//! Strict recovery evidence and single-use consent contracts.

use std::fmt;

use d2b_contracts::v3::{
    CanonicalJsonError, CanonicalJsonValue, canonical_digest, canonical_json_bytes,
};
use serde::{Deserialize, Serialize};

use crate::model::{
    CandidateId, Digest, IdError, OperationId, OperationKind, OperatorId, RecoveryId,
};

/// Domain separator for recovery attestation digests.
pub const RECOVERY_DOMAIN: &str = "d2b:cutover:recovery:v1";
/// Maximum lifetime accepted for one recovery attestation.
pub const MAX_RECOVERY_LIFETIME_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// A qualified external full-host recovery point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryAttestation {
    recovery_id: RecoveryId,
    candidate_id: CandidateId,
    host_digest: Digest,
    preview_digest: Digest,
    operator_id: OperatorId,
    restore_instructions_digest: Digest,
    issued_at_ms: u64,
    expires_at_ms: u64,
    qualified: bool,
}

impl RecoveryAttestation {
    /// Construct a recovery attestation with bounded integer time.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recovery_id: RecoveryId,
        candidate_id: CandidateId,
        host_digest: Digest,
        preview_digest: Digest,
        operator_id: OperatorId,
        restore_instructions_digest: Digest,
        issued_at_ms: u64,
        expires_at_ms: u64,
        qualified: bool,
    ) -> Result<Self, ConsentError> {
        validate_time_window(issued_at_ms, expires_at_ms)?;
        Ok(Self {
            recovery_id,
            candidate_id,
            host_digest,
            preview_digest,
            operator_id,
            restore_instructions_digest,
            issued_at_ms,
            expires_at_ms,
            qualified,
        })
    }

    /// Decode strict canonical evidence JSON.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, ConsentError> {
        CanonicalJsonValue::parse(bytes).map_err(ConsentError::CanonicalJson)?;
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ConsentError::Json)?;
        value.validate_shape()?;
        Ok(value)
    }

    /// Render canonical evidence JSON.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ConsentError> {
        canonical_json_bytes(self).map_err(ConsentError::CanonicalJson)
    }

    /// Compute the digest bound into consent and the journal request.
    pub fn digest(&self) -> Result<Digest, ConsentError> {
        let bytes = self.canonical_bytes()?;
        Ok(Digest::parse(canonical_digest(RECOVERY_DOMAIN, &bytes))?)
    }

    /// Validate the attestation against one exact operation context.
    pub fn validate_for(
        &self,
        expected_candidate: &CandidateId,
        expected_preview: &Digest,
        expected_operator: &OperatorId,
        expected_host: &Digest,
        now_ms: u64,
    ) -> Result<(), ConsentError> {
        if !self.qualified
            || &self.candidate_id != expected_candidate
            || &self.preview_digest != expected_preview
            || &self.operator_id != expected_operator
            || &self.host_digest != expected_host
            || now_ms < self.issued_at_ms
            || now_ms > self.expires_at_ms
        {
            return Err(ConsentError::RecoveryMismatch);
        }
        Ok(())
    }

    /// Borrow the recovery identity.
    pub fn recovery_id(&self) -> &RecoveryId {
        &self.recovery_id
    }

    /// Borrow the bound candidate.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Borrow the bound preview digest.
    pub fn preview_digest(&self) -> &Digest {
        &self.preview_digest
    }

    /// Borrow the bound operator.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Borrow the restore-instructions digest.
    pub fn restore_instructions_digest(&self) -> &Digest {
        &self.restore_instructions_digest
    }

    /// Return the issue time.
    pub const fn issued_at_ms(&self) -> u64 {
        self.issued_at_ms
    }

    /// Return the expiry time.
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    /// Return whether external qualification succeeded.
    pub const fn qualified(&self) -> bool {
        self.qualified
    }

    fn validate_shape(&self) -> Result<(), ConsentError> {
        validate_time_window(self.issued_at_ms, self.expires_at_ms)?;
        if !self.qualified {
            return Err(ConsentError::NotQualified);
        }
        Ok(())
    }
}

/// All values required to bind apply consent to one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsentBinding {
    operation_id: OperationId,
    operation_kind: OperationKind,
    candidate_id: CandidateId,
    preview_digest: Digest,
    recovery_digest: Option<Digest>,
    operator_id: OperatorId,
}

impl ConsentBinding {
    /// Construct an exact binding.
    pub fn new(
        operation_id: OperationId,
        operation_kind: OperationKind,
        candidate_id: CandidateId,
        preview_digest: Digest,
        recovery_digest: Option<Digest>,
        operator_id: OperatorId,
    ) -> Self {
        Self {
            operation_id,
            operation_kind,
            candidate_id,
            preview_digest,
            recovery_digest,
            operator_id,
        }
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Return the operation kind.
    pub const fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    /// Borrow the candidate identity.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Borrow the preview digest.
    pub fn preview_digest(&self) -> &Digest {
        &self.preview_digest
    }

    /// Borrow the optional recovery digest.
    pub fn recovery_digest(&self) -> Option<&Digest> {
        self.recovery_digest.as_ref()
    }

    /// Borrow the bound operator.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }
}

/// One single-use apply consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consent {
    binding: ConsentBinding,
    issued_at_ms: u64,
    expires_at_ms: u64,
    consumed: bool,
}

impl Consent {
    /// Decode one strict canonical consent artifact.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, ConsentError> {
        CanonicalJsonValue::parse(bytes).map_err(ConsentError::CanonicalJson)?;
        let consent: Self = serde_json::from_slice(bytes).map_err(|_| ConsentError::Json)?;
        validate_time_window(consent.issued_at_ms, consent.expires_at_ms)?;
        Ok(consent)
    }

    /// Compute the digest bound to the apply request.
    pub fn digest(&self) -> Result<crate::model::Digest, ConsentError> {
        Ok(crate::model::Digest::derive(
            "d2b:cutover:consent:v1",
            &self.canonical_bytes()?,
        ))
    }

    /// Render canonical consent bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ConsentError> {
        canonical_json_bytes(self).map_err(ConsentError::CanonicalJson)
    }

    /// Issue consent for one exact operation binding.
    pub fn issue(
        binding: ConsentBinding,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<Self, ConsentError> {
        validate_time_window(issued_at_ms, expires_at_ms)?;
        Ok(Self {
            binding,
            issued_at_ms,
            expires_at_ms,
            consumed: false,
        })
    }

    /// Consume consent once after exact binding and expiry validation.
    pub fn consume(&mut self, expected: &ConsentBinding, now_ms: u64) -> Result<(), ConsentError> {
        if self.consumed
            || &self.binding != expected
            || now_ms < self.issued_at_ms
            || now_ms > self.expires_at_ms
        {
            return Err(ConsentError::Invalid);
        }
        self.consumed = true;
        Ok(())
    }

    /// Return whether this consent was already consumed.
    pub const fn is_consumed(&self) -> bool {
        self.consumed
    }

    /// Return the consent issuance time.
    pub const fn issued_at_ms(&self) -> u64 {
        self.issued_at_ms
    }

    /// Borrow the exact binding.
    pub fn binding(&self) -> &ConsentBinding {
        &self.binding
    }

    /// Return the expiry time.
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
}

/// A distinct phase-10 finalization binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationBinding {
    operation_id: OperationId,
    operation_kind: OperationKind,
    candidate_id: CandidateId,
    preview_digest: Digest,
    operator_id: OperatorId,
}

impl FinalizationBinding {
    /// Construct the separate phase-10 binding.
    pub fn new(
        operation_id: OperationId,
        operation_kind: OperationKind,
        candidate_id: CandidateId,
        preview_digest: Digest,
        operator_id: OperatorId,
    ) -> Self {
        Self {
            operation_id,
            operation_kind,
            candidate_id,
            preview_digest,
            operator_id,
        }
    }
}

/// A separately issued, single-use phase-10 consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationConsent {
    binding: FinalizationBinding,
    issued_at_ms: u64,
    expires_at_ms: u64,
    consumed: bool,
}

impl FinalizationConsent {
    /// Decode one strict canonical finalization consent artifact.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, ConsentError> {
        CanonicalJsonValue::parse(bytes).map_err(ConsentError::CanonicalJson)?;
        let consent: Self = serde_json::from_slice(bytes).map_err(|_| ConsentError::Json)?;
        validate_time_window(consent.issued_at_ms, consent.expires_at_ms)?;
        Ok(consent)
    }

    /// Render canonical finalization consent bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ConsentError> {
        canonical_json_bytes(self).map_err(ConsentError::CanonicalJson)
    }

    /// Issue a distinct finalization consent.
    pub fn issue(
        binding: FinalizationBinding,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<Self, ConsentError> {
        validate_time_window(issued_at_ms, expires_at_ms)?;
        Ok(Self {
            binding,
            issued_at_ms,
            expires_at_ms,
            consumed: false,
        })
    }

    /// Compute the digest bound to a phase-10 broker effect.
    pub fn digest(&self) -> Result<Digest, ConsentError> {
        Ok(Digest::derive(
            "d2b:cutover:finalization-consent:v1",
            &self.canonical_bytes()?,
        ))
    }

    /// Consume finalization consent once.
    pub fn consume(
        &mut self,
        expected: &FinalizationBinding,
        now_ms: u64,
    ) -> Result<(), ConsentError> {
        if self.consumed
            || &self.binding != expected
            || now_ms < self.issued_at_ms
            || now_ms > self.expires_at_ms
        {
            return Err(ConsentError::Invalid);
        }
        self.consumed = true;
        Ok(())
    }

    /// Return whether this consent was consumed.
    pub const fn is_consumed(&self) -> bool {
        self.consumed
    }
}

/// Strict evidence or consent failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentError {
    /// Duplicate, fractional, negative, trailing, or malformed JSON.
    CanonicalJson(CanonicalJsonError),
    /// Typed JSON decoding failed.
    Json,
    /// A bounded identifier or digest was invalid.
    Id(IdError),
    /// The time window was outside the bounded ordered range.
    InvalidTime,
    /// The evidence was not externally qualified.
    NotQualified,
    /// The recovery evidence did not match the exact operation.
    RecoveryMismatch,
    /// A consent token was stale, mismatched, expired, or replayed.
    Invalid,
}

impl fmt::Display for ConsentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalJson(_) => "strict canonical JSON rejected",
            Self::Json => "typed evidence JSON rejected",
            Self::Id(_) => "evidence identity rejected",
            Self::InvalidTime => "bounded evidence time rejected",
            Self::NotQualified => "recovery evidence is not qualified",
            Self::RecoveryMismatch => "recovery evidence mismatch",
            Self::Invalid => "consent invalid or already consumed",
        })
    }
}

impl std::error::Error for ConsentError {}

impl From<IdError> for ConsentError {
    fn from(error: IdError) -> Self {
        Self::Id(error)
    }
}

fn validate_time_window(issued_at_ms: u64, expires_at_ms: u64) -> Result<(), ConsentError> {
    if expires_at_ms <= issued_at_ms
        || expires_at_ms - issued_at_ms > MAX_RECOVERY_LIFETIME_MS
        || issued_at_ms > i64::MAX as u64
        || expires_at_ms > i64::MAX as u64
    {
        Err(ConsentError::InvalidTime)
    } else {
        Ok(())
    }
}
