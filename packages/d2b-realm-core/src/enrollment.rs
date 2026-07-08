//! Enrollment and identity-lifecycle metadata for realms.
//!
//! These DTOs carry only opaque references, fingerprints, ids, timestamps, and
//! status metadata. They never carry private keys, public key bytes, provider
//! credentials, signatures, session secrets, or signed credential material.

use crate::capability::Capability;
use crate::ids::{
    ControllerGenerationCredentialRef, ControllerGenerationId, CorrelationId, EnrollmentId,
    KeyRotationId, RealmId, RealmIdentityRef, RecoveryProcedureId, RevocationId, RevocationListId,
    WorkloadId,
};
use crate::realm::RealmPath;
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

const FINGERPRINT_PREFIX: &str = "sha256:";
const FINGERPRINT_HEX_LEN: usize = 64;
const FINGERPRINT_LEN: usize = FINGERPRINT_PREFIX.len() + FINGERPRINT_HEX_LEN;

/// Maximum records carried by one metadata revocation list snapshot.
pub const MAX_REVOCATION_LIST_RECORDS: usize = 512;
/// Maximum realms that may be named in one propagation-status snapshot.
pub const MAX_PROPAGATED_REALMS: usize = 64;
/// Maximum workload ids that may be named in one teardown directive.
pub const MAX_TEARDOWN_WORKLOADS: usize = 64;
/// Maximum recovery evidence references carried by a metadata procedure.
pub const MAX_RECOVERY_EVIDENCE_REFS: usize = 16;

/// Bounded cryptographic fingerprint. This is metadata, not key material.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct KeyFingerprint(String);

impl KeyFingerprint {
    /// Parse `sha256:<64 lowercase hex chars>`.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        parse_fingerprint(raw.into()).map(Self)
    }

    /// Borrow the fingerprint.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for KeyFingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KeyFingerprint(<sha256>)")
    }
}

impl<'de> Deserialize<'de> for KeyFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid key fingerprint"))
    }
}

impl JsonSchema for KeyFingerprint {
    fn schema_name() -> String {
        "KeyFingerprint".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        fingerprint_schema()
    }
}

/// Fingerprint of a realm identity key. This is metadata, not key material.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RealmIdentityFingerprint(String);

impl RealmIdentityFingerprint {
    /// Parse `sha256:<64 lowercase hex chars>`.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        parse_fingerprint(raw.into()).map(Self)
    }

    /// Borrow the fingerprint.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for RealmIdentityFingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RealmIdentityFingerprint(<sha256>)")
    }
}

impl<'de> Deserialize<'de> for RealmIdentityFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid realm identity fingerprint"))
    }
}

impl JsonSchema for RealmIdentityFingerprint {
    fn schema_name() -> String {
        "RealmIdentityFingerprint".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        fingerprint_schema()
    }
}

fn parse_fingerprint(raw: String) -> Option<String> {
    let hex = raw.strip_prefix(FINGERPRINT_PREFIX)?;
    if hex.len() == FINGERPRINT_HEX_LEN
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        Some(raw)
    } else {
        None
    }
}

fn fingerprint_schema() -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            min_length: Some(FINGERPRINT_LEN as u32),
            max_length: Some(FINGERPRINT_LEN as u32),
            pattern: Some("^sha256:[0-9a-f]{64}$".to_owned()),
        })),
        ..Default::default()
    })
}

/// Lifecycle state of a realm identity reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmIdentityStatus {
    Active,
    Rotating,
    Superseded,
    Revoked,
    RecoveryOnly,
}

/// Metadata for a realm identity key. The reference and fingerprint are the
/// only key-shaped values; no key bytes or credential material are modeled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmIdentityMetadata {
    pub realm: RealmPath,
    pub identity_ref: RealmIdentityRef,
    pub fingerprint: RealmIdentityFingerprint,
    pub status: RealmIdentityStatus,
    pub created_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub not_after_unix_seconds: Option<u64>,
}

/// Lifecycle state of a controller-generation credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ControllerGenerationStatus {
    Pending,
    Active,
    Rotating,
    Superseded,
    Revoked,
    Recovering,
}

/// Metadata for a realm controller generation and its credential reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControllerGenerationMetadata {
    pub realm: RealmPath,
    pub generation_id: ControllerGenerationId,
    pub realm_identity: RealmIdentityMetadata,
    pub credential_ref: ControllerGenerationCredentialRef,
    pub credential_fingerprint: KeyFingerprint,
    pub status: ControllerGenerationStatus,
    pub issued_at_unix_seconds: u64,
    pub not_before_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub not_after_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revoked_by: Option<RevocationId>,
}

/// Role of a pinned realm key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmKeyRole {
    /// Long-lived realm identity key.
    RealmIdentity,
    /// Parent realm trust anchor pinned into a child.
    ParentTrustAnchor,
    /// Child realm identity observed and pinned by a parent.
    ChildIdentity,
    /// Controller-generation signing key.
    ControllerGeneration,
}

/// Parent/child key pinning metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyPin {
    /// Realm whose key is pinned.
    pub realm: RealmPath,
    /// Parent or child label at the trust edge.
    pub peer: RealmId,
    /// Key role represented by this fingerprint.
    pub role: RealmKeyRole,
    /// Fingerprint only; no key bytes.
    pub fingerprint: KeyFingerprint,
    /// Controller generation that produced or accepted this pin.
    pub controller_generation: ControllerGenerationId,
    /// Issuance/observation time as Unix seconds.
    pub pinned_at_unix_seconds: u64,
}

/// Parent trust-anchor metadata installed into a child realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParentTrustAnchor {
    pub parent_realm: RealmPath,
    pub child_realm: RealmPath,
    pub parent_identity_ref: RealmIdentityRef,
    pub parent_fingerprint: RealmIdentityFingerprint,
    pub accepted_by_generation: ControllerGenerationId,
    pub pinned_at_unix_seconds: u64,
}

/// Child identity key pinned by a parent during enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildKeyPin {
    pub parent_realm: RealmPath,
    pub child_realm: RealmPath,
    pub child_identity_ref: RealmIdentityRef,
    pub child_fingerprint: RealmIdentityFingerprint,
    pub accepted_by_generation: ControllerGenerationId,
    pub enrollment_id: EnrollmentId,
    pub pinned_at_unix_seconds: u64,
}

/// Enrollment lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnrollmentStatus {
    Pending,
    Accepted,
    Rejected,
    Superseded,
    Revoked,
    RecoveryRequired,
}

/// Stable rejection/supersession reason for enrollment metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnrollmentReason {
    OperatorRequested,
    ParentPolicyDenied,
    ChildKeyMismatch,
    ParentTrustAnchorMismatch,
    ControllerGenerationRevoked,
    RecoveryReplacement,
}

/// Metadata-only enrollment record for a child realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentRecord {
    /// Enrollment record id.
    pub enrollment_id: EnrollmentId,
    /// Parent realm that admits the child.
    pub parent_realm: RealmPath,
    /// Child realm being enrolled.
    pub child_realm: RealmPath,
    /// Active controller generation for this enrollment.
    pub controller_generation: ControllerGenerationMetadata,
    /// Parent trust-anchor pin metadata.
    pub parent_trust_anchor: ParentTrustAnchor,
    /// Child identity pin metadata.
    pub child_key_pin: ChildKeyPin,
    /// Current status.
    pub status: EnrollmentStatus,
    /// Optional stable status reason.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<EnrollmentReason>,
    /// Bounded provider/bootstrap method token.
    pub bootstrap_method: ProtocolToken,
    /// Enrollment creation time.
    pub created_at_unix_seconds: u64,
    /// Last status update time.
    pub updated_at_unix_seconds: u64,
    /// Correlation id for audit across parent/child records.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

/// Key material class controlled by a rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyRotationSubjectKind {
    RealmIdentity,
    ControllerGeneration,
}

/// Key rotation subject metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum KeyRotationSubject {
    RealmIdentity {
        realm: RealmPath,
        current_identity_ref: RealmIdentityRef,
        current_fingerprint: RealmIdentityFingerprint,
    },
    ControllerGeneration {
        realm: RealmPath,
        current_generation: ControllerGenerationId,
        current_credential_ref: ControllerGenerationCredentialRef,
        current_fingerprint: KeyFingerprint,
    },
}

impl KeyRotationSubject {
    pub fn kind(&self) -> KeyRotationSubjectKind {
        match self {
            Self::RealmIdentity { .. } => KeyRotationSubjectKind::RealmIdentity,
            Self::ControllerGeneration { .. } => KeyRotationSubjectKind::ControllerGeneration,
        }
    }
}

/// Stable reason for a key rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyRotationReason {
    Routine,
    OperatorRequested,
    SuspectedCompromise,
    ParentRequested,
    Recovery,
    AlgorithmMigration,
}

/// Lifecycle status for a key rotation plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyRotationStatus {
    Planned,
    Issued,
    Active,
    Superseded,
    Revoked,
    Failed,
}

/// Metadata-only rotation plan. Replacement fields are opaque refs and
/// fingerprints only; no generated key bytes or signatures are represented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyRotationPlan {
    pub rotation_id: KeyRotationId,
    pub realm: RealmPath,
    pub subject: KeyRotationSubject,
    pub reason: KeyRotationReason,
    pub status: KeyRotationStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replacement_identity_ref: Option<RealmIdentityRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replacement_credential_ref: Option<ControllerGenerationCredentialRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replacement_fingerprint: Option<KeyFingerprint>,
    pub planned_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub activate_after_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

/// Stable key-rotation audit/event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyRotationEventKind {
    Planned,
    CredentialIssued,
    Activated,
    Superseded,
    Revoked,
    Failed,
}

/// Metadata-only key-rotation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyRotationEvent {
    pub rotation_id: KeyRotationId,
    pub realm: RealmPath,
    pub subject_kind: KeyRotationSubjectKind,
    pub event: KeyRotationEventKind,
    pub status: KeyRotationStatus,
    pub observed_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

/// Thing revoked by a parent or realm controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RevocationTarget {
    Realm {
        realm: RealmPath,
    },
    RealmKey {
        realm: RealmPath,
        role: RealmKeyRole,
        fingerprint: KeyFingerprint,
    },
    RealmIdentity {
        realm: RealmPath,
        identity_ref: RealmIdentityRef,
        fingerprint: RealmIdentityFingerprint,
    },
    ControllerGeneration {
        realm: RealmPath,
        controller_generation: ControllerGenerationId,
    },
    ControllerCredential {
        realm: RealmPath,
        credential_ref: ControllerGenerationCredentialRef,
        fingerprint: KeyFingerprint,
    },
    Enrollment {
        enrollment_id: EnrollmentId,
    },
    PolicyGrant {
        realm: RealmPath,
        grant: ProtocolToken,
    },
    CapabilityGrant {
        realm: RealmPath,
        capability: Capability,
    },
}

/// Stable revocation reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationReason {
    OperatorRequested,
    KeyCompromised,
    ControllerCompromised,
    ParentPolicyRevoked,
    EnrollmentSuperseded,
    RecoveryCompleted,
    Expired,
}

/// Revocation lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationStatus {
    Pending,
    Effective,
    Propagating,
    Propagated,
    Superseded,
}

/// Metadata-only revocation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevocationRecord {
    /// Revocation record id.
    pub revocation_id: RevocationId,
    /// Realm issuing the revocation.
    pub issuer_realm: RealmPath,
    /// Issuer controller generation.
    pub issuer_controller_generation: ControllerGenerationId,
    /// Target metadata.
    pub target: RevocationTarget,
    /// Current status.
    pub status: RevocationStatus,
    /// Low-cardinality reason code.
    pub reason: RevocationReason,
    /// Issue time.
    pub issued_at_unix_seconds: u64,
    /// Optional effective time.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub effective_at_unix_seconds: Option<u64>,
    /// Cross-realm audit correlation id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

/// Parent-pushed revocation-list propagation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationListStatus {
    Draft,
    Published,
    Propagating,
    Propagated,
    Superseded,
}

/// Metadata-only parent-pushed revocation-list snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevocationList {
    pub list_id: RevocationListId,
    pub issuer_realm: RealmPath,
    pub issuer_controller_generation: ControllerGenerationId,
    pub status: RevocationListStatus,
    #[schemars(length(min = 1, max = 512))]
    pub records: Vec<RevocationRecord>,
    #[schemars(length(max = 64))]
    pub propagated_to: Vec<RealmPath>,
    pub generated_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub supersedes: Option<RevocationListId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

impl<'de> Deserialize<'de> for RevocationList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            list_id: RevocationListId,
            issuer_realm: RealmPath,
            issuer_controller_generation: ControllerGenerationId,
            status: RevocationListStatus,
            #[serde(default)]
            records: Vec<RevocationRecord>,
            #[serde(default)]
            propagated_to: Vec<RealmPath>,
            generated_at_unix_seconds: u64,
            #[serde(default)]
            supersedes: Option<RevocationListId>,
            #[serde(default)]
            correlation_id: Option<CorrelationId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.records.is_empty() || raw.records.len() > MAX_REVOCATION_LIST_RECORDS {
            return Err(serde::de::Error::custom(
                "revocation list must carry 1..=512 records",
            ));
        }
        if raw.propagated_to.len() > MAX_PROPAGATED_REALMS {
            return Err(serde::de::Error::custom(
                "revocation list propagated_to exceeds 64 realms",
            ));
        }
        Ok(Self {
            list_id: raw.list_id,
            issuer_realm: raw.issuer_realm,
            issuer_controller_generation: raw.issuer_controller_generation,
            status: raw.status,
            records: raw.records,
            propagated_to: raw.propagated_to,
            generated_at_unix_seconds: raw.generated_at_unix_seconds,
            supersedes: raw.supersedes,
            correlation_id: raw.correlation_id,
        })
    }
}

/// Runtime session teardown cause metadata. This only describes what must be
/// torn down; it does not implement routing, transport, or process cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SessionTeardownReason {
    RealmIdentityRevoked,
    ControllerGenerationRevoked,
    PolicyGrantRevoked,
    StreamCapabilityRevoked,
    RecoveryIsolation,
}

/// Metadata-only directive to terminate sessions depending on a revoked grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionTeardownDirective {
    pub revocation_id: RevocationId,
    pub issuer_realm: RealmPath,
    pub affected_realm: RealmPath,
    pub reason: SessionTeardownReason,
    #[schemars(length(max = 64))]
    pub affected_workloads: Vec<WorkloadId>,
    pub issued_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

impl<'de> Deserialize<'de> for SessionTeardownDirective {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            revocation_id: RevocationId,
            issuer_realm: RealmPath,
            affected_realm: RealmPath,
            reason: SessionTeardownReason,
            #[serde(default)]
            affected_workloads: Vec<WorkloadId>,
            issued_at_unix_seconds: u64,
            #[serde(default)]
            correlation_id: Option<CorrelationId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.affected_workloads.len() > MAX_TEARDOWN_WORKLOADS {
            return Err(serde::de::Error::custom(
                "session teardown directive exceeds 64 workloads",
            ));
        }
        Ok(Self {
            revocation_id: raw.revocation_id,
            issuer_realm: raw.issuer_realm,
            affected_realm: raw.affected_realm,
            reason: raw.reason,
            affected_workloads: raw.affected_workloads,
            issued_at_unix_seconds: raw.issued_at_unix_seconds,
            correlation_id: raw.correlation_id,
        })
    }
}

/// Stable recovery cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryReason {
    LostControllerKey,
    CompromisedControllerKey,
    ParentInitiatedReset,
    OperatorBreakGlass,
}

/// Metadata lifecycle status for child-controller recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryStatus {
    Requested,
    ParentApproved,
    Isolating,
    Reissued,
    Completed,
    Rejected,
}

/// Metadata-only recovery procedure for a lost or compromised child controller
/// key. Evidence values are bounded opaque refs, not logs or secret bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryProcedure {
    pub recovery_id: RecoveryProcedureId,
    pub parent_realm: RealmPath,
    pub child_realm: RealmPath,
    pub reason: RecoveryReason,
    pub status: RecoveryStatus,
    pub affected_generation: ControllerGenerationId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replacement_generation: Option<ControllerGenerationMetadata>,
    #[schemars(length(max = 16))]
    pub evidence_refs: Vec<ProtocolToken>,
    pub opened_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub closed_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

impl<'de> Deserialize<'de> for RecoveryProcedure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            recovery_id: RecoveryProcedureId,
            parent_realm: RealmPath,
            child_realm: RealmPath,
            reason: RecoveryReason,
            status: RecoveryStatus,
            affected_generation: ControllerGenerationId,
            #[serde(default)]
            replacement_generation: Option<ControllerGenerationMetadata>,
            #[serde(default)]
            evidence_refs: Vec<ProtocolToken>,
            opened_at_unix_seconds: u64,
            #[serde(default)]
            closed_at_unix_seconds: Option<u64>,
            #[serde(default)]
            correlation_id: Option<CorrelationId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.evidence_refs.len() > MAX_RECOVERY_EVIDENCE_REFS {
            return Err(serde::de::Error::custom(
                "recovery procedure exceeds 16 evidence refs",
            ));
        }
        Ok(Self {
            recovery_id: raw.recovery_id,
            parent_realm: raw.parent_realm,
            child_realm: raw.child_realm,
            reason: raw.reason,
            status: raw.status,
            affected_generation: raw.affected_generation,
            replacement_generation: raw.replacement_generation,
            evidence_refs: raw.evidence_refs,
            opened_at_unix_seconds: raw.opened_at_unix_seconds,
            closed_at_unix_seconds: raw.closed_at_unix_seconds,
            correlation_id: raw.correlation_id,
        })
    }
}

/// Low-cardinality identity-lifecycle audit event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityAuditEventKind {
    EnrollmentRequested,
    EnrollmentAccepted,
    EnrollmentRejected,
    RotationPlanned,
    RotationActivated,
    RevocationIssued,
    RevocationPropagated,
    RecoveryOpened,
    RecoveryCompleted,
}

/// Metadata attached to audit records for enrollment/rotation/revocation and
/// recovery. It names only ids, realm paths, statuses, and reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityAuditEventMetadata {
    pub event: IdentityAuditEventKind,
    pub realm: RealmPath,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub enrollment_id: Option<EnrollmentId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rotation_id: Option<KeyRotationId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revocation_id: Option<RevocationId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recovery_id: Option<RecoveryProcedureId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub enrollment_status: Option<EnrollmentStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rotation_status: Option<KeyRotationStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revocation_status: Option<RevocationStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recovery_status: Option<RecoveryStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn key_fp() -> KeyFingerprint {
        KeyFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap()
    }

    fn identity_fp() -> RealmIdentityFingerprint {
        RealmIdentityFingerprint::parse(format!("sha256:{}", "b".repeat(64))).unwrap()
    }

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    fn identity_metadata() -> RealmIdentityMetadata {
        RealmIdentityMetadata {
            realm: realm("work"),
            identity_ref: RealmIdentityRef::parse("idref-1").unwrap(),
            fingerprint: identity_fp(),
            status: RealmIdentityStatus::Active,
            created_at_unix_seconds: 1,
            not_after_unix_seconds: None,
        }
    }

    fn controller_generation() -> ControllerGenerationMetadata {
        ControllerGenerationMetadata {
            realm: realm("work"),
            generation_id: ControllerGenerationId::parse("gen-1").unwrap(),
            realm_identity: identity_metadata(),
            credential_ref: ControllerGenerationCredentialRef::parse("cgref-1").unwrap(),
            credential_fingerprint: key_fp(),
            status: ControllerGenerationStatus::Active,
            issued_at_unix_seconds: 2,
            not_before_unix_seconds: 2,
            not_after_unix_seconds: None,
            revoked_by: None,
        }
    }

    fn revocation_record() -> RevocationRecord {
        RevocationRecord {
            revocation_id: RevocationId::parse("rev-1").unwrap(),
            issuer_realm: realm("local-root"),
            issuer_controller_generation: ControllerGenerationId::parse("gen-2").unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: realm("work"),
                controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            },
            status: RevocationStatus::Effective,
            reason: RevocationReason::ControllerCompromised,
            issued_at_unix_seconds: 20,
            effective_at_unix_seconds: Some(21),
            correlation_id: None,
        }
    }

    fn enrollment_record() -> EnrollmentRecord {
        EnrollmentRecord {
            enrollment_id: EnrollmentId::parse("enroll-1").unwrap(),
            parent_realm: realm("local-root"),
            child_realm: realm("work"),
            controller_generation: controller_generation(),
            parent_trust_anchor: ParentTrustAnchor {
                parent_realm: realm("local-root"),
                child_realm: realm("work"),
                parent_identity_ref: RealmIdentityRef::parse("idref-parent").unwrap(),
                parent_fingerprint: identity_fp(),
                accepted_by_generation: ControllerGenerationId::parse("gen-1").unwrap(),
                pinned_at_unix_seconds: 10,
            },
            child_key_pin: ChildKeyPin {
                parent_realm: realm("local-root"),
                child_realm: realm("work"),
                child_identity_ref: RealmIdentityRef::parse("idref-child").unwrap(),
                child_fingerprint: identity_fp(),
                accepted_by_generation: ControllerGenerationId::parse("gen-1").unwrap(),
                enrollment_id: EnrollmentId::parse("enroll-1").unwrap(),
                pinned_at_unix_seconds: 10,
            },
            status: EnrollmentStatus::Accepted,
            reason: None,
            bootstrap_method: ProtocolToken::parse("host-local").unwrap(),
            created_at_unix_seconds: 10,
            updated_at_unix_seconds: 11,
            correlation_id: Some(CorrelationId::parse("corr-1").unwrap()),
        }
    }

    #[test]
    fn fingerprints_reject_key_material_shapes() {
        assert!(KeyFingerprint::parse(format!("sha256:{}", "a".repeat(64))).is_some());
        assert!(RealmIdentityFingerprint::parse(format!("sha256:{}", "b".repeat(64))).is_some());
        assert!(KeyFingerprint::parse("-----BEGIN PUBLIC KEY-----").is_none());
        assert!(RealmIdentityFingerprint::parse(format!("sha256:{}", "A".repeat(64))).is_none());
        assert_eq!(format!("{:?}", key_fp()), "KeyFingerprint(<sha256>)");
        assert_eq!(
            format!("{:?}", identity_fp()),
            "RealmIdentityFingerprint(<sha256>)"
        );
    }

    #[test]
    fn refs_reject_secret_shaped_strings_and_redact_debug() {
        assert!(RealmIdentityRef::parse("idref-1").is_ok());
        assert!(ControllerGenerationCredentialRef::parse("cgref-1").is_ok());
        assert!(RealmIdentityRef::parse("secret-identity").is_err());
        assert!(ControllerGenerationCredentialRef::parse("credential-material").is_err());
        assert!(ControllerGenerationCredentialRef::parse("-----BEGIN-PRIVATE-KEY-----").is_err());

        let metadata = controller_generation();
        let debug = format!("{metadata:?}");
        assert!(!debug.contains("cgref-1"));
        assert!(!debug.contains(identity_fp().as_str()));
        assert!(debug.contains("ControllerGenerationCredentialRef(<7 bytes>)"));
    }

    #[test]
    fn enrollment_record_rejects_unknown_fields() {
        let mut value = serde_json::to_value(enrollment_record()).unwrap();
        value.as_object_mut().unwrap().insert(
            "privateKey".to_owned(),
            serde_json::Value::String("must-not-appear".to_owned()),
        );
        assert!(serde_json::from_value::<EnrollmentRecord>(value).is_err());
    }

    #[test]
    fn revocation_list_decode_enforces_bounds() {
        let list = RevocationList {
            list_id: RevocationListId::parse("rvlist-1").unwrap(),
            issuer_realm: realm("local-root"),
            issuer_controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            status: RevocationListStatus::Published,
            records: vec![revocation_record()],
            propagated_to: vec![realm("work")],
            generated_at_unix_seconds: 30,
            supersedes: None,
            correlation_id: None,
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(serde_json::from_str::<RevocationList>(&json).is_ok());

        let mut empty = serde_json::to_value(&list).unwrap();
        empty
            .as_object_mut()
            .unwrap()
            .insert("records".to_owned(), serde_json::Value::Array(vec![]));
        assert!(serde_json::from_value::<RevocationList>(empty).is_err());

        let mut too_many = serde_json::to_value(&list).unwrap();
        too_many.as_object_mut().unwrap().insert(
            "propagatedTo".to_owned(),
            serde_json::Value::Array(vec![serde_json::to_value(realm("work")).unwrap(); 65]),
        );
        assert!(serde_json::from_value::<RevocationList>(too_many).is_err());
    }

    #[test]
    fn teardown_and_recovery_decode_enforce_bounds() {
        let directive = SessionTeardownDirective {
            revocation_id: RevocationId::parse("rev-1").unwrap(),
            issuer_realm: realm("local-root"),
            affected_realm: realm("work"),
            reason: SessionTeardownReason::ControllerGenerationRevoked,
            affected_workloads: vec![WorkloadId::parse("build-vm").unwrap()],
            issued_at_unix_seconds: 40,
            correlation_id: None,
        };
        assert!(
            serde_json::from_str::<SessionTeardownDirective>(
                &serde_json::to_string(&directive).unwrap()
            )
            .is_ok()
        );

        let mut too_many_workloads = serde_json::to_value(&directive).unwrap();
        too_many_workloads.as_object_mut().unwrap().insert(
            "affectedWorkloads".to_owned(),
            serde_json::Value::Array(
                (0..65)
                    .map(|i| serde_json::Value::String(format!("vm-{i}")))
                    .collect(),
            ),
        );
        assert!(serde_json::from_value::<SessionTeardownDirective>(too_many_workloads).is_err());

        let recovery = RecoveryProcedure {
            recovery_id: RecoveryProcedureId::parse("recover-1").unwrap(),
            parent_realm: realm("local-root"),
            child_realm: realm("work"),
            reason: RecoveryReason::LostControllerKey,
            status: RecoveryStatus::Requested,
            affected_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            replacement_generation: None,
            evidence_refs: vec![],
            opened_at_unix_seconds: 50,
            closed_at_unix_seconds: None,
            correlation_id: None,
        };
        let mut too_many_evidence = serde_json::to_value(&recovery).unwrap();
        too_many_evidence.as_object_mut().unwrap().insert(
            "evidenceRefs".to_owned(),
            serde_json::Value::Array(
                (0..17)
                    .map(|i| serde_json::Value::String(format!("evidence-{i}")))
                    .collect(),
            ),
        );
        assert!(serde_json::from_value::<RecoveryProcedure>(too_many_evidence).is_err());
    }

    #[test]
    fn status_and_reason_enums_are_stable_kebab_case() {
        assert_eq!(
            serde_json::to_string(&EnrollmentStatus::RecoveryRequired).unwrap(),
            "\"recovery-required\""
        );
        assert_eq!(
            serde_json::to_string(&KeyRotationReason::SuspectedCompromise).unwrap(),
            "\"suspected-compromise\""
        );
        assert_eq!(
            serde_json::to_string(&RevocationStatus::Propagating).unwrap(),
            "\"propagating\""
        );
        assert_eq!(
            serde_json::to_string(&RecoveryStatus::ParentApproved).unwrap(),
            "\"parent-approved\""
        );
    }

    #[test]
    fn identity_audit_metadata_carries_ids_not_material() {
        let event = IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RevocationIssued,
            realm: realm("work"),
            enrollment_id: None,
            rotation_id: Some(KeyRotationId::parse("rotate-1").unwrap()),
            revocation_id: Some(RevocationId::parse("rev-1").unwrap()),
            recovery_id: None,
            enrollment_status: None,
            rotation_status: Some(KeyRotationStatus::Revoked),
            revocation_status: Some(RevocationStatus::Effective),
            recovery_status: None,
            correlation_id: Some(CorrelationId::parse("corr-1").unwrap()),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("revocation-issued"));
        assert!(!json.contains("private"));
        assert!(!json.contains("BEGIN"));
        assert!(serde_json::from_str::<IdentityAuditEventMetadata>(&json).is_ok());
    }

    #[test]
    fn identity_lifecycle_schemas_are_generated() {
        for schema in [
            schema_for!(RealmIdentityMetadata).schema,
            schema_for!(ControllerGenerationMetadata).schema,
            schema_for!(EnrollmentRecord).schema,
            schema_for!(KeyRotationPlan).schema,
            schema_for!(RevocationList).schema,
            schema_for!(SessionTeardownDirective).schema,
            schema_for!(RecoveryProcedure).schema,
            schema_for!(IdentityAuditEventMetadata).schema,
        ] {
            assert!(schema.metadata.is_some());
        }
    }
}
