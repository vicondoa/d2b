//! Common authority identity, freshness, and refusal contracts.
//!
//! One vocabulary is spoken by every admission and effect surface: the
//! subject that initiated a request, the store incarnation and desired
//! revision the request was formed against, the stage that refused, and the
//! reason it refused. Those values are evidence to be evaluated, never a
//! decision: nothing here grants access, resolves a host path, names a
//! numeric host principal, or carries secret material.
//!
//! The desired revision is deliberately distinct from the spec generation a
//! status row reports. A committed desired mutation advances the revision
//! even when the rendered spec bytes are unchanged, so an ownership, view,
//! consumer, provider-assignment, or execution-policy change cannot leave an
//! earlier effect's authority looking current.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
    },
    identity::{ResourceTypeName, ResourceUid, ZoneId},
    resource_schema::{framed_canonical_digest, is_canonical_digest},
};
use d2b_contracts::wire_deserialize;

/// Maximum bytes in one store-incarnation token.
pub const MAX_STORE_INCARNATION_BYTES: usize = 63;
/// The domain tag framing the canonical digest of one committed desired row.
pub const DESIRED_ROW_DIGEST_DOMAIN_TAG: &str = "d2b:v3:desired-row";

/// The identity of one durable store generation.
///
/// An incarnation is an identity, never an ordered counter: a snapshot or
/// commit naming a different incarnation than the broker last durably
/// accepted is a different store, not a newer one, and installing it requires
/// the explicit ownership-bounded reset rather than ordinary acceptance.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StoreIncarnation(BoundedToken);

impl StoreIncarnation {
    /// Parse a bounded lower-kebab store incarnation.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.len() > MAX_STORE_INCARNATION_BYTES {
            return Err(PrimitiveSpecError::InvalidToken);
        }
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical incarnation string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(StoreIncarnation);
parsed_deserialize!(StoreIncarnation);
string_schema!(StoreIncarnation, 1, MAX_STORE_INCARNATION_BYTES);

/// A monotonic counter that fails closed instead of wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionError {
    /// The counter reached its ceiling; no wraparound is permitted.
    Exhausted,
}

impl core::fmt::Display for RevisionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("desired revision space is exhausted")
    }
}

impl std::error::Error for RevisionError {}

/// One row's desired revision.
///
/// Every committed desired mutation - spec, owner, metadata, or deletion -
/// advances it, and an identical ensure does not. Runtime status never
/// advances it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DesiredRevision(u64);

impl DesiredRevision {
    /// The revision a freshly created row starts at.
    pub const INITIAL: Self = Self(0);

    /// The revision value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advance one revision, or refuse when the counter would wrap.
    pub const fn try_next(self) -> Result<Self, RevisionError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(RevisionError::Exhausted),
        }
    }
}

impl JsonSchema for DesiredRevision {
    fn schema_name() -> String {
        "DesiredRevision".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::Integer,
            ))),
            ..Default::default()
        };
        schema.number().minimum = Some(0.0);
        schemars::schema::Schema::Object(schema)
    }
}

/// The Zone-wide desired sequence.
///
/// The sequence orders committed desired mutations within one store
/// incarnation and is what a broker fences its projection against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZoneDesiredSequence(u64);

impl ZoneDesiredSequence {
    /// The sequence a freshly initialized Zone starts at.
    pub const INITIAL: Self = Self(0);

    /// The sequence value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advance one sequence, or refuse when the counter would wrap.
    pub const fn try_next(self) -> Result<Self, RevisionError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(RevisionError::Exhausted),
        }
    }
}

impl JsonSchema for ZoneDesiredSequence {
    fn schema_name() -> String {
        "ZoneDesiredSequence".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::Integer,
            ))),
            ..Default::default()
        };
        schema.number().minimum = Some(0.0);
        schemars::schema::Schema::Object(schema)
    }
}

/// The canonical digest of one committed desired row.
///
/// The digest is non-secret freshness data, not a bearer credential: it
/// identifies which bytes were committed, never the right to act on them.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DesiredDigest(String);

impl DesiredDigest {
    /// Parse a framed canonical digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err(PrimitiveSpecError::InvalidText)
        }
    }

    /// Frame the canonical digest of committed desired bytes.
    pub fn of(canonical_bytes: &[u8]) -> Self {
        Self(framed_canonical_digest(
            DESIRED_ROW_DIGEST_DOMAIN_TAG,
            canonical_bytes,
        ))
    }

    /// Borrow the framed digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(DesiredDigest);
parsed_deserialize!(DesiredDigest);
string_schema!(DesiredDigest, 1, 128);

/// The exact desired state one admitted effect is fenced against.
///
/// A provider assignment, source view, target, execution policy, or
/// RoleBinding dependency is part of the effect's dependency set: comparing
/// this tuple, not the spec generation a status row happens to report, is
/// what decides whether earlier authority still holds.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshnessTuple {
    zone: ZoneId,
    store_incarnation: StoreIncarnation,
    resource_ref: ResourceRef,
    resource_uid: ResourceUid,
    desired_revision: DesiredRevision,
    desired_digest: DesiredDigest,
}

impl FreshnessTuple {
    /// Construct the tuple naming one committed row in one store generation.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        zone: ZoneId,
        store_incarnation: StoreIncarnation,
        resource_ref: ResourceRef,
        resource_uid: ResourceUid,
        desired_revision: DesiredRevision,
        desired_digest: DesiredDigest,
    ) -> Self {
        Self {
            zone,
            store_incarnation,
            resource_ref,
            resource_uid,
            desired_revision,
            desired_digest,
        }
    }

    /// Borrow the Zone the row belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store generation.
    pub const fn store_incarnation(&self) -> &StoreIncarnation {
        &self.store_incarnation
    }

    /// Borrow the exact resource reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the store-assigned row identity.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    /// Borrow the row's desired revision.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }

    /// Borrow the digest of the committed desired bytes.
    pub const fn desired_digest(&self) -> &DesiredDigest {
        &self.desired_digest
    }

    /// Whether this tuple names the same store generation as `other`.
    ///
    /// A different generation is a different store, not a newer one, so the
    /// comparison fails rather than ordering.
    pub fn same_store(&self, other: &Self) -> bool {
        self.zone == other.zone && self.store_incarnation == other.store_incarnation
    }
}

redacted_debug!(FreshnessTuple);

wire_deserialize!(
    FreshnessTuple,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        zone: ZoneId,
        store_incarnation: StoreIncarnation,
        resource_ref: ResourceRef,
        resource_uid: ResourceUid,
        desired_revision: DesiredRevision,
        desired_digest: DesiredDigest,
    },
    wire,
    Ok(FreshnessTuple::new(
        wire.zone,
        wire.store_incarnation,
        wire.resource_ref,
        wire.resource_uid,
        wire.desired_revision,
        wire.desired_digest,
    ))
);

/// The class of subject that initiated a request.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AuthoritySubjectKind {
    /// An admitted User identity.
    User,
    /// A long-running Process.
    Process,
    /// A run-to-completion EphemeralProcess.
    EphemeralProcess,
    /// A Host execution target.
    Host,
    /// A Guest execution target.
    Guest,
    /// A provider component acting under its own admitted identity.
    Provider,
    /// The verified deployment bootstrap graph.
    Bootstrap,
    /// Explicit local operator authority for a one-shot ownership action.
    Operator,
}

/// The canonical `Guest` ResourceType name.
///
/// The authority contract classifies a `Guest` reference as a subject, so
/// the name it compares against belongs here rather than only in the crate
/// that declares the type's base spec.
pub const GUEST_RESOURCE_TYPE: &str = "Guest";

impl AuthoritySubjectKind {
    /// The subject class one resource type carries, when it has one.
    ///
    /// The classification reads the canonical type-name constants instead of
    /// restating the vocabulary, so no shared crate holds a private copy of
    /// the type names it decides on. A type outside this set - a `Zone`
    /// self-resource, a group, a link - has no admitted subject, and the
    /// caller refuses it rather than classifying it as something it is not.
    pub fn of_resource_type(candidate: &ResourceTypeName) -> Option<Self> {
        let name = candidate.as_str();
        if name == super::host::HOST_RESOURCE_TYPE {
            Some(Self::Host)
        } else if name == super::process::PROCESS_RESOURCE_TYPE {
            Some(Self::Process)
        } else if name == super::process::EPHEMERAL_PROCESS_RESOURCE_TYPE {
            Some(Self::EphemeralProcess)
        } else if name == super::user::USER_RESOURCE_TYPE {
            Some(Self::User)
        } else if name == GUEST_RESOURCE_TYPE {
            Some(Self::Guest)
        } else if name == super::operation::PROVIDER_RESOURCE_TYPE {
            Some(Self::Provider)
        } else {
            None
        }
    }

    /// The subject class one reference carries, when it has one.
    pub fn of_reference(reference: &ResourceRef) -> Option<Self> {
        Self::of_resource_type(reference.resource_type())
    }
}

/// The subject an authority decision is made for.
///
/// A privileged transport identity is never a subject of its own: a nested
/// call keeps the initiating subject, so arriving through a broker or
/// session connection cannot substitute a more privileged identity for the
/// one that started the work.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthoritySubject {
    kind: AuthoritySubjectKind,
    resource_ref: Option<ResourceRef>,
}

impl AuthoritySubject {
    /// Construct a subject that names its exact resource.
    pub const fn named(kind: AuthoritySubjectKind, resource_ref: ResourceRef) -> Self {
        Self {
            kind,
            resource_ref: Some(resource_ref),
        }
    }

    /// Construct a subject that names no resource.
    ///
    /// Only the verified deployment graph and explicit local operator
    /// authority are unresourced; neither is a workload identity.
    pub const fn unresourced(kind: AuthoritySubjectKind) -> Self {
        Self {
            kind,
            resource_ref: None,
        }
    }

    /// Borrow the subject class.
    pub const fn kind(&self) -> AuthoritySubjectKind {
        self.kind
    }

    /// Borrow the exact resource, when the subject names one.
    pub const fn resource_ref(&self) -> Option<&ResourceRef> {
        self.resource_ref.as_ref()
    }

    /// Whether the subject is the verified deployment graph or explicit
    /// local operator authority rather than a workload identity.
    pub const fn is_bootstrap_class(&self) -> bool {
        matches!(
            self.kind,
            AuthoritySubjectKind::Bootstrap | AuthoritySubjectKind::Operator
        )
    }
}

redacted_debug!(AuthoritySubject);

wire_deserialize!(
    AuthoritySubject,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        kind: AuthoritySubjectKind,
        resource_ref: Option<ResourceRef>,
    },
    wire,
    {
        if wire.resource_ref.is_none()
            && !matches!(
                wire.kind,
                AuthoritySubjectKind::Bootstrap | AuthoritySubjectKind::Operator
            )
        {
            return Err(serde::de::Error::custom(
                "only bootstrap and operator subjects may omit a resource",
            ));
        }
        Ok(AuthoritySubject {
            kind: wire.kind,
            resource_ref: wire.resource_ref,
        })
    }
);

/// The stage a decision was made at.
///
/// The stage is what makes a failure attributable: it names where in the
/// lifecycle the request stopped, so an operator sees the enforcing stage
/// rather than an unexplained launch mismatch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdmissionStage {
    /// The desired declaration is being normalized and deduplicated.
    Normalize,
    /// The subject's authorization for the requested relationship.
    Authorize,
    /// The source provider's decision on the exact request.
    Admit,
    /// The source-side physical or logical reservation.
    Reserve,
    /// Delivery prerequisites being established.
    Prepare,
    /// The consumer starting to use a prepared relationship.
    Activate,
    /// New use being blocked ahead of typed release.
    Revoke,
    /// Outstanding use being driven to its declared safe state.
    Drain,
    /// The relationship being finalized and its reservation released.
    Release,
    /// Restart or interrupted-effect recovery.
    Recover,
}

impl AdmissionStage {
    /// Whether reaching this stage admits new use.
    ///
    /// A control action taken at one of the last three stages can only
    /// reduce use or recover known state; it can never create a binding, a
    /// consumer-serving child privilege, or a source claim.
    pub const fn admits_new_use(self) -> bool {
        matches!(
            self,
            Self::Authorize | Self::Admit | Self::Reserve | Self::Prepare | Self::Activate
        )
    }
}

/// One typed refusal reason.
///
/// Every variant is field-free, so a diagnostic can never echo a path, a
/// resource identity, or caller-supplied text. The resource, relationship,
/// and enforcing stage travel beside the reason, never inside it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalReason {
    /// The subject is not authorized for the requested policy selection.
    PolicySelectionNotAuthorized,
    /// A required namespace class is outside the admitted policy.
    RequiredNamespaceNotAdmitted,
    /// A required capability is outside the admitted ceiling.
    RequiredCapabilityOutsideCeiling,
    /// Instance input attempted to weaken a mandatory restriction.
    RestrictionWeakened,
    /// The requested identity is not authorized by the admitted rules.
    IdentityNotAuthorized,
    /// A mandatory confinement facet is not enforceable on this target.
    MandatoryFacetUnsupported,
    /// The selected profile does not admit the implementation's syscall needs.
    SeccompIncompatible,
    /// The requested limit exceeds an admitted ceiling.
    LimitExceedsCeiling,
    /// An admitted emergency reduction is blocking new use.
    ///
    /// A Zone's `EmergencyPolicy` row is a committed policy reduction, so a
    /// request that arrives while it is enforced is refused at the revoking
    /// stage even though the same request was admitted against the earlier
    /// graph. The reason is its own so an operator can tell a policy
    /// reduction from a quota ceiling; the flag itself stays in the policy
    /// row, never in the refusal.
    EmergencyReductionActive,
    /// The target support ceiling does not admit the requested capability.
    TargetSupportMissing,
    /// The requested relationship conflicts with an existing declaration.
    ConflictingDeclaration,
    /// The source's own policy refused the request.
    SourcePolicyRefused,
    /// The effect was fenced by a newer accepted revision.
    StaleAuthority,
    /// The store generation or sequence does not match accepted authority.
    StoreIncarnationMismatch,
    /// Recovery could not prove that an effect is complete or absent.
    UnprovenEffect,
    /// The requested implementation is not a trusted declared identity.
    UntrustedImplementation,
}

/// One admission outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum AdmissionDecision {
    /// The exact requested relationship is authorized against current evidence.
    Admitted,
    /// The request was refused at one stage for one reason.
    Refused {
        /// The stage that refused.
        stage: AdmissionStage,
        /// Why it refused.
        reason: RefusalReason,
    },
}

impl AdmissionDecision {
    /// Whether the decision admitted the request.
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }

    /// Build a refusal carrying its enforcing stage.
    pub const fn refuse(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self::Refused { stage, reason }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    const DIGEST_INPUT: &[u8] = br#"{"spec":{"kind":"host"}}"#;

    fn tuple() -> FreshnessTuple {
        FreshnessTuple::new(
            ZoneId::parse("work").expect("zone"),
            StoreIncarnation::parse("store-1").expect("incarnation"),
            ResourceRef::parse("Host/studio").expect("host ref"),
            ResourceUid::parse(ROW_UID).expect("uid"),
            DesiredRevision::INITIAL.try_next().expect("revision"),
            DesiredDigest::of(DIGEST_INPUT),
        )
    }

    #[test]
    fn revisions_advance_and_refuse_to_wrap() {
        let first = DesiredRevision::INITIAL;
        let second = first.try_next().expect("advance");
        assert_eq!(second.get(), 1);
        assert_eq!(ZoneDesiredSequence::INITIAL.try_next().expect("advance").get(), 1);
        assert_eq!(
            DesiredRevision(u64::MAX).try_next(),
            Err(RevisionError::Exhausted)
        );
        assert_eq!(
            ZoneDesiredSequence(u64::MAX).try_next(),
            Err(RevisionError::Exhausted)
        );
    }

    #[test]
    fn a_store_incarnation_is_an_identity_not_an_order() {
        let left = tuple();
        let right = FreshnessTuple::new(
            ZoneId::parse("work").expect("zone"),
            StoreIncarnation::parse("store-2").expect("incarnation"),
            ResourceRef::parse("Host/studio").expect("host ref"),
            ResourceUid::parse(ROW_UID).expect("uid"),
            left.desired_revision(),
            DesiredDigest::of(DIGEST_INPUT),
        );
        assert!(!left.same_store(&right));
        assert!(left.same_store(&tuple()));
    }

    #[test]
    fn the_digest_is_framed_and_validated() {
        let digest = DesiredDigest::of(DIGEST_INPUT);
        assert!(digest.as_str().starts_with("sha256:"));
        assert_eq!(DesiredDigest::parse(digest.as_str()), Ok(digest.clone()));
        assert_eq!(
            DesiredDigest::parse("not-a-digest"),
            Err(PrimitiveSpecError::InvalidText)
        );
        assert_ne!(
            DesiredDigest::of(br#"{"spec":{"kind":"guest"}}"#).as_str(),
            digest.as_str()
        );
    }

    #[test]
    fn only_bootstrap_and_operator_subjects_may_omit_a_resource() {
        assert!(
            serde_json::from_str::<AuthoritySubject>(
                r#"{"kind":"bootstrap","resourceRef":null}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<AuthoritySubject>(r#"{"kind":"user","resourceRef":null}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<AuthoritySubject>(r#"{"kind":"user"}"#).is_err(),
            "a workload subject must name its exact resource"
        );
    }

    #[test]
    fn a_transport_identity_cannot_become_the_subject() {
        let subject = AuthoritySubject::named(
            AuthoritySubjectKind::Process,
            ResourceRef::parse("Process/web").expect("process ref"),
        );
        assert_eq!(subject.kind(), AuthoritySubjectKind::Process);
        assert!(!subject.is_bootstrap_class());
        assert_eq!(
            subject
                .resource_ref()
                .map(ResourceRef::to_canonical_string)
                .as_deref(),
            Some("Process/web")
        );
    }

    #[test]
    fn only_the_ordinary_stages_admit_new_use() {
        assert!(AdmissionStage::Authorize.admits_new_use());
        assert!(AdmissionStage::Prepare.admits_new_use());
        for stage in [
            AdmissionStage::Revoke,
            AdmissionStage::Drain,
            AdmissionStage::Release,
            AdmissionStage::Recover,
        ] {
            assert!(!stage.admits_new_use(), "{stage:?} must only reduce use");
        }
    }

    #[test]
    fn a_refusal_names_its_enforcing_stage() {
        let decision = AdmissionDecision::refuse(
            AdmissionStage::Authorize,
            RefusalReason::PolicySelectionNotAuthorized,
        );
        assert!(!decision.is_admitted());
        assert_eq!(
            decision,
            AdmissionDecision::Refused {
                stage: AdmissionStage::Authorize,
                reason: RefusalReason::PolicySelectionNotAuthorized,
            }
        );
    }

    #[test]
    fn the_freshness_tuple_rejects_an_unknown_field() {
        let json = format!(
            r#"{{"zone":"work","storeIncarnation":"store-1","resourceRef":"Host/studio","resourceUid":"{ROW_UID}","desiredRevision":1,"desiredDigest":"{}","specGeneration":4}}"#,
            DesiredDigest::of(DIGEST_INPUT).as_str()
        );
        assert!(serde_json::from_str::<FreshnessTuple>(&json).is_err());
    }
}
