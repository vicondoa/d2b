//! Strict recovery-point attestation and candidate-bound delivery closure.

use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_contracts::v3::{CanonicalJsonError, CanonicalJsonValue, canonical_json_bytes};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as DeserializeError, Visitor},
};
use sha2::{Digest as ShaDigest, Sha256};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result, SnapshotView,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    evidence::{self, EvidenceLane, EvidenceRecord, OutputDigest},
    model::{CandidateId, ContentId, SnapshotSha256, validate_identifier, validate_sha256},
    storage::{CandidateDir, MAX_JSON_BYTES, StateRoot},
};

/// The exact external recovery attestation artifact kind.
pub const RECOVERY_ATTESTATION_ARTIFACT_KIND: &str = "d2b-recovery-point-attestation";
/// The external recovery attestation schema version.
pub const RECOVERY_ATTESTATION_SCHEMA_VERSION: u32 = 1;
/// The d2b release program named by the attestation.
pub const RECOVERY_PROGRAM: &str = "ADR046";
/// Explicit release spelling accepted for externally authored records.
pub const RECOVERY_PROGRAM_RELEASE: &str = "d2b-3.0";
/// Domain used when binding canonical recovery-attestation bytes.
pub const RECOVERY_ATTESTATION_DOMAIN: &str = "d2b:recovery-attestation:v1";
/// Domain used for host identity digests.
pub const RECOVERY_HOST_DOMAIN: &str = "d2b:recovery-host:v1";
/// Domain used for operator subject digests.
pub const RECOVERY_OPERATOR_DOMAIN: &str = "d2b:recovery-operator:v1";
/// Domain used for opaque recovery locator digests.
pub const RECOVERY_LOCATOR_DOMAIN: &str = "d2b:recovery-point-locator:v1";
/// Domain used for restore-instruction digests.
pub const RECOVERY_RESTORE_DOMAIN: &str = "d2b:recovery-restore-instructions:v1";
/// Domain used for pinned closure store path digests.
pub const CLOSURE_STORE_PATH_DOMAIN: &str = "d2b:delivery:closure-store-path:v1";
/// Maximum representable recovery timestamp.
pub const MAX_RECOVERY_UNIX_SECONDS: u64 = 253_402_300_799;
/// Required retention deadline from capture and verification.
pub const RECOVERY_DEADLINE_SECONDS: u64 = 86_400;
/// Validation identifier used by the shared candidate evidence reader.
pub const RECOVERY_EVIDENCE_VALIDATION: &str = "recovery-point-attestation";

/// Candidate-addressed delivery record names.
pub const BINDING_REQUEST_FILE: &str = "binding-request.json";
pub const TERMINAL_FAILURE_FILE: &str = "terminal-failure.json";
pub const CUTOVER_RESULT_FILE: &str = "cutover-result.json";
pub const MERGE_ATTEMPT_FILE: &str = "merge-attempt.json";
pub const POST_MERGE_RECONCILIATION_FILE: &str = "post-merge-reconciliation.json";
pub const POST_MERGE_SEAL_FILE: &str = "post-merge-seal.json";
pub const FINALIZATION_FILE: &str = "finalization.json";
pub const CLOSE_FILE: &str = "close.json";

const BINDING_REQUEST_ARTIFACT_KIND: &str = "d2b-delivery/binding-request";
const TERMINAL_FAILURE_ARTIFACT_KIND: &str = "d2b-delivery/terminal-failure";
const CUTOVER_RESULT_ARTIFACT_KIND: &str = "d2b-delivery/cutover-result";
const MERGE_ATTEMPT_ARTIFACT_KIND: &str = "d2b-delivery/merge-attempt";
const POST_MERGE_RECONCILIATION_ARTIFACT_KIND: &str = "d2b-delivery/post-merge-reconciliation";
const POST_MERGE_SEAL_ARTIFACT_KIND: &str = "d2b-delivery/post-merge-seal";
const FINALIZATION_ARTIFACT_KIND: &str = "d2b-delivery/finalization";
const CLOSE_ARTIFACT_KIND: &str = "d2b-delivery/close";

/// A bounded UTC timestamp used by the recovery contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, JsonSchema)]
#[schemars(transparent)]
#[schemars(range(max = 253402300799))]
pub struct RecoveryUnixSeconds(u64);

impl RecoveryUnixSeconds {
    /// Construct a timestamp inside the closed recovery range.
    pub fn new(value: u64) -> RecoveryResult<Self> {
        if value <= MAX_RECOVERY_UNIX_SECONDS {
            Ok(Self(value))
        } else {
            Err(RecoveryError::Timestamp)
        }
    }

    /// Return the bounded integer value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    fn checked_add(self, seconds: u64) -> RecoveryResult<Self> {
        let value = self
            .0
            .checked_add(seconds)
            .ok_or(RecoveryError::Timestamp)?;
        Self::new(value)
    }
}

impl Serialize for RecoveryUnixSeconds {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for RecoveryUnixSeconds {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SecondsVisitor;

        impl Visitor<'_> for SecondsVisitor {
            type Value = RecoveryUnixSeconds;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded unsigned integer number of Unix seconds")
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: DeserializeError,
            {
                RecoveryUnixSeconds::new(value).map_err(E::custom)
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
            where
                E: DeserializeError,
            {
                if value < 0 {
                    Err(E::custom("negative recovery timestamp"))
                } else {
                    self.visit_u64(value as u64)
                }
            }
        }

        deserializer.deserialize_u64(SecondsVisitor)
    }
}

/// A lowercase SHA-256 digest without a raw locator or path.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd, JsonSchema)]
#[schemars(regex(pattern = "^[a-f0-9]{64}$"))]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Parse a lowercase hexadecimal SHA-256 digest.
    pub fn parse(value: impl Into<String>) -> RecoveryResult<Self> {
        let value = value.into();
        validate_sha256_value(&value)?;
        Ok(Self(value))
    }

    /// Borrow the digest spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Sha256Digest(<redacted>)")
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A full lowercase Git object ID.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd, JsonSchema)]
#[schemars(regex(pattern = "^(?:[a-f0-9]{40}|[a-f0-9]{64})$"))]
pub struct GitObjectId(String);

impl GitObjectId {
    /// Parse a full SHA-1 or SHA-256 Git object ID.
    pub fn parse(value: impl Into<String>) -> RecoveryResult<Self> {
        let value = value.into();
        if !matches!(value.len(), 40 | 64)
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RecoveryError::Binding);
        }
        Ok(Self(value))
    }

    /// Borrow the object ID.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GitObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GitObjectId(<redacted>)")
    }
}

impl fmt::Display for GitObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for GitObjectId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for GitObjectId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Recovery point kind accepted by the attestation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryPointKind {
    /// An external full-host snapshot.
    FullHostSnapshot,
    /// An external full-host backup.
    FullHostBackup,
}

/// External verification mechanism accepted by the attestation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationMethod {
    /// Snapshot provider readback.
    SnapshotReadback,
    /// Backup provider verification.
    BackupVerify,
}

/// The only accepted verification result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationResult {
    /// The external provider verified the point.
    Passed,
}

/// The only accepted final attestation result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttestationResult {
    /// The recovery point qualifies for delivery.
    Passed,
}

/// Closed qualification booleans for a full-host recovery point.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryQualification {
    /// Boot and system state are covered.
    pub boot_and_system_state_covered: bool,
    /// The exact preview inventory is covered.
    pub affected_artifact_inventory_covered: bool,
    /// Preserved identity state is covered.
    pub preserved_identity_state_covered: bool,
    /// The point restores to this same host.
    pub same_host_restore_target: bool,
    /// The point remains read-only through expiry.
    pub read_only_until_expiry: bool,
}

impl RecoveryQualification {
    fn validate(&self) -> RecoveryResult<()> {
        if self.boot_and_system_state_covered
            && self.affected_artifact_inventory_covered
            && self.preserved_identity_state_covered
            && self.same_host_restore_target
            && self.read_only_until_expiry
        {
            Ok(())
        } else {
            Err(RecoveryError::Qualification)
        }
    }
}

/// The strict external recovery-point attestation.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryAttestation {
    /// Fixed artifact kind.
    pub artifact_kind: String,
    /// Fixed version of the external attestation.
    pub schema_version: u32,
    /// Release program identifier.
    pub program: String,
    /// Candidate content identity.
    pub candidate_id: CandidateId,
    /// Exact release commit.
    pub commit_oid: GitObjectId,
    /// Exact release tree.
    pub tree_oid: GitObjectId,
    /// Digest of the pinned closure store path, never the path itself.
    pub closure_store_path_sha256: Sha256Digest,
    /// Opaque bundle generation identity.
    pub bundle_generation: String,
    /// Digest of the same-host identity.
    pub host_identity_sha256: Sha256Digest,
    /// Digest of the operator subject.
    pub operator_subject_sha256: Sha256Digest,
    /// Digest of the canonical preview bytes.
    pub preview_sha256: Sha256Digest,
    /// External recovery point kind.
    pub recovery_point_kind: RecoveryPointKind,
    /// Digest of the opaque external locator.
    pub recovery_point_locator_sha256: Sha256Digest,
    /// Digest of the exact restore instructions.
    pub restore_instructions_sha256: Sha256Digest,
    /// Time the preview was produced.
    pub previewed_at_unix: RecoveryUnixSeconds,
    /// Time the external point was captured.
    pub captured_at_unix: RecoveryUnixSeconds,
    /// Time the point was externally verified.
    pub verified_at_unix: RecoveryUnixSeconds,
    /// Time the operator attested to the point.
    pub attested_at_unix: RecoveryUnixSeconds,
    /// Read-only retention deadline.
    pub retention_until_unix: RecoveryUnixSeconds,
    /// Exact derived expiry.
    pub expires_at_unix: RecoveryUnixSeconds,
    /// External verification method.
    pub verification_method: VerificationMethod,
    /// External verification outcome.
    pub verification_result: VerificationResult,
    /// Closed qualification set.
    pub qualification: RecoveryQualification,
    /// Final attestation outcome.
    pub result: AttestationResult,
}

impl fmt::Debug for RecoveryAttestation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryAttestation(<redacted>)")
    }
}

impl RecoveryAttestation {
    /// Decode through the shared duplicate-rejecting canonical JSON parser.
    pub fn decode_json(bytes: &[u8]) -> RecoveryResult<Self> {
        if bytes.len() > MAX_JSON_BYTES {
            return Err(RecoveryError::TooLarge);
        }
        CanonicalJsonValue::parse(bytes).map_err(RecoveryError::CanonicalJson)?;
        let value: Self = serde_json::from_slice(bytes).map_err(|_| RecoveryError::Json)?;
        value.validate_shape()?;
        Ok(value)
    }

    /// Render canonical JSON bytes for the record.
    pub fn canonical_bytes(&self) -> RecoveryResult<Vec<u8>> {
        canonical_json_bytes(self).map_err(RecoveryError::CanonicalJson)
    }

    /// Compute the domain-separated digest of canonical record bytes.
    pub fn digest(&self) -> RecoveryResult<Sha256Digest> {
        let bytes = self.canonical_bytes()?;
        Ok(domain_digest(RECOVERY_ATTESTATION_DOMAIN, &bytes))
    }

    /// Validate the attestation against one exact candidate binding and one
    /// sampled verifier time.
    pub fn validate_at(
        &self,
        binding: &RecoveryBinding,
        verifier_now: u64,
        required_remaining_ttl: u64,
    ) -> RecoveryResult<RecoveryValidation> {
        self.validate_shape()?;
        let verifier_now = RecoveryUnixSeconds::new(verifier_now)?;
        self.validate_binding(binding)?;
        if self.previewed_at_unix > self.captured_at_unix
            || self.captured_at_unix > self.verified_at_unix
            || self.verified_at_unix > self.attested_at_unix
            || self.attested_at_unix > verifier_now
            || verifier_now >= self.expires_at_unix
        {
            return Err(RecoveryError::Freshness);
        }
        let remaining = self.expires_at_unix.as_u64() - verifier_now.as_u64();
        if remaining < required_remaining_ttl {
            return Err(RecoveryError::InsufficientTtl);
        }
        Ok(RecoveryValidation {
            attestation_sha256: Sha256Digest::parse(super::model::sha256_bytes(
                &self.canonical_bytes()?,
            ))?,
            expires_at: self.expires_at_unix,
            remaining_ttl_seconds: remaining,
        })
    }

    /// Validate while sampling the verifier clock exactly once.
    pub fn validate_with_clock(
        &self,
        binding: &RecoveryBinding,
        required_remaining_ttl: u64,
        sample_clock: impl FnOnce() -> u64,
    ) -> RecoveryResult<RecoveryValidation> {
        let verifier_now = sample_clock();
        self.validate_at(binding, verifier_now, required_remaining_ttl)
    }

    /// Validate using the system clock, sampled once for this invocation.
    pub fn validate_now(
        &self,
        binding: &RecoveryBinding,
        required_remaining_ttl: u64,
    ) -> RecoveryResult<RecoveryValidation> {
        let verifier_now = sampled_unix_seconds()?;
        self.validate_at(binding, verifier_now, required_remaining_ttl)
    }

    fn validate_shape(&self) -> RecoveryResult<()> {
        if self.artifact_kind != RECOVERY_ATTESTATION_ARTIFACT_KIND
            || self.schema_version != RECOVERY_ATTESTATION_SCHEMA_VERSION
            || !matches!(
                self.program.as_str(),
                RECOVERY_PROGRAM | RECOVERY_PROGRAM_RELEASE
            )
        {
            return Err(RecoveryError::Shape);
        }
        validate_bundle_generation(&self.bundle_generation)?;
        if self.recovery_point_kind == RecoveryPointKind::FullHostSnapshot
            && self.verification_method != VerificationMethod::SnapshotReadback
        {
            return Err(RecoveryError::Shape);
        }
        if self.recovery_point_kind == RecoveryPointKind::FullHostBackup
            && self.verification_method != VerificationMethod::BackupVerify
        {
            return Err(RecoveryError::Shape);
        }
        if self.verification_result != VerificationResult::Passed
            || self.result != AttestationResult::Passed
        {
            return Err(RecoveryError::Qualification);
        }
        self.qualification.validate()?;
        let captured_deadline = self
            .captured_at_unix
            .checked_add(RECOVERY_DEADLINE_SECONDS)?;
        let verified_deadline = self
            .verified_at_unix
            .checked_add(RECOVERY_DEADLINE_SECONDS)?;
        let expected_expiry = captured_deadline
            .min(verified_deadline)
            .min(self.retention_until_unix);
        if self.expires_at_unix != expected_expiry {
            return Err(RecoveryError::Expiry);
        }
        Ok(())
    }

    fn validate_binding(&self, binding: &RecoveryBinding) -> RecoveryResult<()> {
        if self.candidate_id != binding.candidate_id
            || self.commit_oid != binding.commit_oid
            || self.tree_oid != binding.tree_oid
            || self.closure_store_path_sha256 != binding.closure_store_path_sha256
            || self.bundle_generation != binding.bundle_generation
            || self.preview_sha256 != binding.preview_sha256
            || self.host_identity_sha256 != binding.host_identity_sha256
            || self.operator_subject_sha256 != binding.operator_subject_sha256
            || self.restore_instructions_sha256 != binding.restore_instructions_sha256
            || self.recovery_point_locator_sha256 != binding.recovery_point_locator_sha256
        {
            return Err(RecoveryError::Binding);
        }
        Ok(())
    }
}

/// The release identity a recovery attestation must match.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryBinding {
    /// Candidate identity.
    pub candidate_id: CandidateId,
    /// Exact release commit.
    pub commit_oid: GitObjectId,
    /// Exact release tree.
    pub tree_oid: GitObjectId,
    /// Pinned closure path digest.
    pub closure_store_path_sha256: Sha256Digest,
    /// Pinned bundle generation.
    pub bundle_generation: String,
    /// Canonical preview digest.
    pub preview_sha256: Sha256Digest,
    /// Host identity digest.
    pub host_identity_sha256: Sha256Digest,
    /// Operator subject digest.
    pub operator_subject_sha256: Sha256Digest,
    /// Restore instruction digest.
    pub restore_instructions_sha256: Sha256Digest,
    /// Digest of the opaque recovery locator.
    pub recovery_point_locator_sha256: Sha256Digest,
}

impl fmt::Debug for RecoveryBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryBinding(<redacted>)")
    }
}

impl RecoveryBinding {
    /// Construct and validate one exact release binding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        candidate_id: CandidateId,
        commit_oid: impl Into<String>,
        tree_oid: impl Into<String>,
        closure_store_path_sha256: Sha256Digest,
        bundle_generation: impl Into<String>,
        preview_sha256: Sha256Digest,
        host_identity_sha256: Sha256Digest,
        operator_subject_sha256: Sha256Digest,
        restore_instructions_sha256: Sha256Digest,
        recovery_point_locator_sha256: Sha256Digest,
    ) -> RecoveryResult<Self> {
        let binding = Self {
            candidate_id,
            commit_oid: GitObjectId::parse(commit_oid)?,
            tree_oid: GitObjectId::parse(tree_oid)?,
            closure_store_path_sha256,
            bundle_generation: bundle_generation.into(),
            preview_sha256,
            host_identity_sha256,
            operator_subject_sha256,
            restore_instructions_sha256,
            recovery_point_locator_sha256,
        };
        validate_bundle_generation(&binding.bundle_generation)?;
        Ok(binding)
    }

    #[cfg(test)]
    fn test_fixture() -> Self {
        Self::new(
            CandidateId::parse("a".repeat(64)).expect("candidate"),
            "b".repeat(40),
            "c".repeat(40),
            Sha256Digest::parse("d".repeat(64)).expect("closure"),
            "generation-1",
            Sha256Digest::parse("1".repeat(64)).expect("preview"),
            Sha256Digest::parse("e".repeat(64)).expect("host"),
            Sha256Digest::parse("f".repeat(64)).expect("operator"),
            Sha256Digest::parse("3".repeat(64)).expect("restore"),
            Sha256Digest::parse("2".repeat(64)).expect("locator"),
        )
        .expect("binding")
    }
}

/// Result of a successful recovery validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryValidation {
    attestation_sha256: Sha256Digest,
    expires_at: RecoveryUnixSeconds,
    remaining_ttl_seconds: u64,
}

impl RecoveryValidation {
    /// Digest of the exact canonical attestation record.
    pub fn attestation_sha256(&self) -> &Sha256Digest {
        &self.attestation_sha256
    }

    /// Exact expiry timestamp.
    pub const fn expires_at(&self) -> RecoveryUnixSeconds {
        self.expires_at
    }

    /// Remaining lifetime at validation time.
    pub const fn remaining_ttl_seconds(&self) -> u64 {
        self.remaining_ttl_seconds
    }
}

/// A pinned closure and its protected GC root, held out-of-band from records.
pub struct FrozenClosure {
    store_path: PathBuf,
    gc_root: PathBuf,
    store_path_sha256: Sha256Digest,
    bundle_generation: String,
}

impl fmt::Debug for FrozenClosure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FrozenClosure(<redacted>)")
    }
}

impl FrozenClosure {
    /// Construct an out-of-band closure binding. Raw paths never serialize.
    pub fn new(
        store_path: impl Into<PathBuf>,
        gc_root: impl Into<PathBuf>,
        store_path_sha256: Sha256Digest,
        bundle_generation: impl Into<String>,
    ) -> RecoveryResult<Self> {
        let closure = Self {
            store_path: store_path.into(),
            gc_root: gc_root.into(),
            store_path_sha256,
            bundle_generation: bundle_generation.into(),
        };
        validate_bundle_generation(&closure.bundle_generation)?;
        Ok(closure)
    }

    /// Validate the pinned path, protected GC root, and path digest.
    pub fn validate(&self) -> RecoveryResult<()> {
        if !self.store_path.is_dir() || !self.gc_root.exists() {
            return Err(RecoveryError::ClosureUnavailable);
        }
        let expected = digest_store_path(&self.store_path)?;
        if expected != self.store_path_sha256 {
            return Err(RecoveryError::ClosureMismatch);
        }
        let store =
            fs::canonicalize(&self.store_path).map_err(|_| RecoveryError::ClosureUnavailable)?;
        let root =
            fs::canonicalize(&self.gc_root).map_err(|_| RecoveryError::ClosureUnavailable)?;
        if root != store {
            return Err(RecoveryError::ClosureMismatch);
        }
        Ok(())
    }

    /// Check that an attestation binding names this exact protected closure.
    pub fn validate_for(&self, binding: &RecoveryBinding) -> RecoveryResult<()> {
        self.validate()?;
        if self.store_path_sha256 != binding.closure_store_path_sha256
            || self.bundle_generation != binding.bundle_generation
        {
            return Err(RecoveryError::ClosureMismatch);
        }
        Ok(())
    }
}

/// Reasons that become immutable after the binding request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalFailureReason {
    /// Required evidence was not unanimous.
    Nonunanimous,
    /// Recovery evidence expired or lacked required remaining lifetime.
    Expired,
    /// Candidate content changed.
    ContentDrift,
    /// Candidate history changed.
    HistoryDrift,
    /// Merge target changed.
    TargetDrift,
    /// Bound evidence changed.
    EvidenceDrift,
    /// Cutover did not complete.
    CutoverFailed,
    /// Merge result or resulting tree did not match.
    MergeMismatch,
    /// Closure or attestation validation failed.
    RecoveryInvalid,
}

/// Durable delivery state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DurableDeliveryState {
    /// No binding request has been published.
    Converging,
    /// The sole binding request exists.
    Bound,
    /// Cutover succeeded and delivery continues.
    CutoverSucceeded,
    /// A merge attempt has been recorded.
    MergeAttempted,
    /// The exact post-merge tree was sealed.
    PostMergeSealed,
    /// Irreversible finalization completed.
    Finalized,
    /// Close completed.
    Closed,
    /// One terminal failure exists.
    Failed,
}

/// Common candidate identity carried by every durable delivery record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryBinding {
    /// Delivery program.
    pub program: String,
    /// Delivery wave.
    pub wave: String,
    /// Candidate content identity.
    pub candidate_id: CandidateId,
    /// Candidate integrated content digest.
    pub content_id: ContentId,
    /// Candidate snapshot/history digest.
    pub snapshot_sha256: SnapshotSha256,
    /// Canonical recovery record digest.
    pub recovery_attestation_sha256: Sha256Digest,
    /// Release commit.
    pub commit_oid: GitObjectId,
    /// Release tree.
    pub tree_oid: GitObjectId,
    /// Pinned closure store path digest.
    pub closure_store_path_sha256: Sha256Digest,
    /// Pinned bundle generation.
    pub bundle_generation: String,
    /// Canonical preview digest.
    pub preview_sha256: Sha256Digest,
    /// Host identity digest.
    pub host_identity_sha256: Sha256Digest,
    /// Operator subject digest.
    pub operator_subject_sha256: Sha256Digest,
    /// Restore instruction digest.
    pub restore_instructions_sha256: Sha256Digest,
    /// Digest of the opaque recovery locator.
    pub recovery_point_locator_sha256: Sha256Digest,
}

impl fmt::Debug for DeliveryBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeliveryBinding(<redacted>)")
    }
}

impl DeliveryBinding {
    /// Construct the delivery identity from the current snapshot and
    /// validated recovery attestation.
    pub fn from_snapshot(
        snapshot: &SnapshotView,
        attestation: &RecoveryAttestation,
        recovery_binding: &RecoveryBinding,
    ) -> RecoveryResult<Self> {
        if attestation.candidate_id != snapshot.candidate_id
            || attestation.program != snapshot.program()
        {
            return Err(RecoveryError::Binding);
        }
        attestation.validate_shape()?;
        attestation.validate_binding(recovery_binding)?;
        let recovery_attestation_sha256 = attestation.digest()?;
        Ok(Self {
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            recovery_attestation_sha256,
            commit_oid: recovery_binding.commit_oid.clone(),
            tree_oid: recovery_binding.tree_oid.clone(),
            closure_store_path_sha256: recovery_binding.closure_store_path_sha256.clone(),
            bundle_generation: recovery_binding.bundle_generation.clone(),
            preview_sha256: recovery_binding.preview_sha256.clone(),
            host_identity_sha256: recovery_binding.host_identity_sha256.clone(),
            operator_subject_sha256: recovery_binding.operator_subject_sha256.clone(),
            restore_instructions_sha256: recovery_binding.restore_instructions_sha256.clone(),
            recovery_point_locator_sha256: recovery_binding.recovery_point_locator_sha256.clone(),
        })
    }

    /// Compute the identity digest used by durable state records.
    pub fn digest(&self) -> RecoveryResult<Sha256Digest> {
        Ok(domain_digest(
            "d2b:delivery:candidate-binding:v1",
            &canonical_json_bytes(self).map_err(RecoveryError::CanonicalJson)?,
        ))
    }
}

/// Binding request record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRequestRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Caller-provided remaining lifetime requirement.
    pub required_remaining_ttl_seconds: u64,
    /// Publication time.
    pub requested_at_unix: RecoveryUnixSeconds,
}

impl fmt::Debug for BindingRequestRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BindingRequestRecord(<redacted>)")
    }
}

impl BindingRequestRecord {
    /// Construct a binding request.
    pub fn new(
        binding: DeliveryBinding,
        required_remaining_ttl_seconds: u64,
        requested_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: BINDING_REQUEST_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            required_remaining_ttl_seconds,
            requested_at_unix: RecoveryUnixSeconds::new(requested_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the record's fixed shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            BINDING_REQUEST_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )?;
        Ok(())
    }
}

/// Terminal failure record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalFailureRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Immutable failure reason.
    pub reason: TerminalFailureReason,
    /// Publication time.
    pub failed_at_unix: RecoveryUnixSeconds,
}

impl fmt::Debug for TerminalFailureRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalFailureRecord(<redacted>)")
    }
}

impl TerminalFailureRecord {
    /// Construct a terminal failure.
    pub fn new(
        binding: DeliveryBinding,
        reason: TerminalFailureReason,
        failed_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: TERMINAL_FAILURE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            reason,
            failed_at_unix: RecoveryUnixSeconds::new(failed_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            TERMINAL_FAILURE_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Cutover result outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverResultStatus {
    /// Phase 0-9 cutover and verification succeeded.
    Succeeded,
    /// Cutover failed and must become terminal.
    Failed,
}

/// Candidate-bound cutover result record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverResultRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Cutover result.
    pub result: CutoverResultStatus,
    /// Publication time.
    pub verified_at_unix: RecoveryUnixSeconds,
}

impl CutoverResultRecord {
    /// Construct a cutover result.
    pub fn new(
        binding: DeliveryBinding,
        result: CutoverResultStatus,
        verified_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: CUTOVER_RESULT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            result,
            verified_at_unix: RecoveryUnixSeconds::new(verified_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            CUTOVER_RESULT_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Merge attempt outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeAttemptStatus {
    /// The expected-head guarded merge succeeded.
    Succeeded,
    /// GitHub returned an ambiguous result.
    Ambiguous,
    /// The merge result did not match the candidate.
    Mismatch,
    /// The merge failed.
    Failed,
}

/// Candidate-bound merge attempt record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeAttemptRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Expected reviewed head digest.
    pub expected_head_sha256: Sha256Digest,
    /// Observed merge result head digest.
    pub observed_head_sha256: Sha256Digest,
    /// Merge result.
    pub result: MergeAttemptStatus,
    /// Publication time.
    pub attempted_at_unix: RecoveryUnixSeconds,
}

impl MergeAttemptRecord {
    /// Construct a merge attempt record.
    pub fn new(
        binding: DeliveryBinding,
        expected_head_sha256: Sha256Digest,
        observed_head_sha256: Sha256Digest,
        result: MergeAttemptStatus,
        attempted_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: MERGE_ATTEMPT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            expected_head_sha256,
            observed_head_sha256,
            result,
            attempted_at_unix: RecoveryUnixSeconds::new(attempted_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            MERGE_ATTEMPT_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Candidate-bound post-merge reconciliation record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostMergeReconciliationRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Tree the candidate approved.
    pub expected_tree_oid: GitObjectId,
    /// Tree observed on `v3`.
    pub observed_tree_oid: GitObjectId,
    /// Whether the two trees are exactly equal.
    pub exact_tree: bool,
    /// Publication time.
    pub reconciled_at_unix: RecoveryUnixSeconds,
}

impl PostMergeReconciliationRecord {
    /// Construct a post-merge reconciliation record.
    pub fn new(
        binding: DeliveryBinding,
        expected_tree_oid: impl Into<String>,
        observed_tree_oid: impl Into<String>,
        reconciled_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let expected_tree_oid = GitObjectId::parse(expected_tree_oid)?;
        let observed_tree_oid = GitObjectId::parse(observed_tree_oid)?;
        let record = Self {
            artifact_kind: POST_MERGE_RECONCILIATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            exact_tree: expected_tree_oid == observed_tree_oid,
            binding,
            expected_tree_oid,
            observed_tree_oid,
            reconciled_at_unix: RecoveryUnixSeconds::new(reconciled_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            POST_MERGE_RECONCILIATION_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Candidate-bound exact post-merge seal.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostMergeSealRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Exact observed tree.
    pub tree_oid: GitObjectId,
    /// Publication time.
    pub sealed_at_unix: RecoveryUnixSeconds,
}

impl PostMergeSealRecord {
    /// Construct an exact post-merge seal.
    pub fn new(
        binding: DeliveryBinding,
        tree_oid: impl Into<String>,
        sealed_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: POST_MERGE_SEAL_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            tree_oid: GitObjectId::parse(tree_oid)?,
            sealed_at_unix: RecoveryUnixSeconds::new(sealed_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            POST_MERGE_SEAL_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Candidate-bound finalization record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Digest of the separate finalization consent.
    pub consent_sha256: Sha256Digest,
    /// Publication time.
    pub finalized_at_unix: RecoveryUnixSeconds,
}

impl FinalizationRecord {
    /// Construct a finalization record.
    pub fn new(
        binding: DeliveryBinding,
        consent_sha256: Sha256Digest,
        finalized_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: FINALIZATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            consent_sha256,
            finalized_at_unix: RecoveryUnixSeconds::new(finalized_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            FINALIZATION_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Candidate-bound close record.
#[derive(Clone, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseRecord {
    /// Record kind.
    pub artifact_kind: String,
    /// Delivery state schema.
    pub schema_version: u32,
    /// Complete candidate binding.
    pub binding: DeliveryBinding,
    /// Digest of the residue audit result.
    pub residue_sha256: Sha256Digest,
    /// Publication time.
    pub closed_at_unix: RecoveryUnixSeconds,
}

impl CloseRecord {
    /// Construct a close record.
    pub fn new(
        binding: DeliveryBinding,
        residue_sha256: Sha256Digest,
        closed_at_unix: u64,
    ) -> RecoveryResult<Self> {
        let record = Self {
            artifact_kind: CLOSE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            residue_sha256,
            closed_at_unix: RecoveryUnixSeconds::new(closed_at_unix)?,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the fixed record shape.
    pub fn validate(&self) -> RecoveryResult<()> {
        validate_record_header(
            &self.artifact_kind,
            CLOSE_ARTIFACT_KIND,
            self.schema_version,
            &self.binding,
        )
    }
}

/// Candidate-addressed durable state writer.
pub struct DeliveryLedger<'a> {
    candidate: &'a CandidateDir,
}

impl<'a> DeliveryLedger<'a> {
    /// Attach the ledger to an existing candidate directory.
    pub const fn new(candidate: &'a CandidateDir) -> Self {
        Self { candidate }
    }

    /// Publish the sole binding request.
    pub fn publish_binding_request(&self, record: &BindingRequestRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_no_terminal()?;
        self.candidate.write_json_once(BINDING_REQUEST_FILE, record)
    }

    /// Validate and publish the sole binding request for a candidate.
    pub fn bind(
        &self,
        snapshot: &SnapshotView,
        attestation: &RecoveryAttestation,
        recovery_binding: &RecoveryBinding,
        verifier_now: u64,
        required_remaining_ttl: u64,
    ) -> Result<RecoveryValidation> {
        self.ensure_replacement_allowed()?;
        snapshot.validate(self.candidate)?;
        let current: SnapshotView = self.candidate.read_json(super::storage::SNAPSHOT_FILE)?;
        if current != *snapshot {
            return Err(DeliveryError::new(
                "candidate snapshot changed before binding; reconverge and revalidate",
            ));
        }
        let validation = attestation
            .validate_at(recovery_binding, verifier_now, required_remaining_ttl)
            .map_err(to_delivery_error)?;
        let binding = DeliveryBinding::from_snapshot(snapshot, attestation, recovery_binding)
            .map_err(to_delivery_error)?;
        self.publish_binding_request(
            &BindingRequestRecord::new(binding, required_remaining_ttl, verifier_now)
                .map_err(to_delivery_error)?,
        )?;
        Ok(validation)
    }

    /// Validate a protected closure before publishing the sole binding
    /// request. Closure failure remains prebinding convergence.
    pub fn bind_with_closure(
        &self,
        snapshot: &SnapshotView,
        attestation: &RecoveryAttestation,
        recovery_binding: &RecoveryBinding,
        closure: &FrozenClosure,
        verifier_now: u64,
        required_remaining_ttl: u64,
    ) -> Result<RecoveryValidation> {
        closure
            .validate_for(recovery_binding)
            .map_err(to_delivery_error)?;
        self.bind(
            snapshot,
            attestation,
            recovery_binding,
            verifier_now,
            required_remaining_ttl,
        )
    }

    /// Publish the one terminal failure. It is never replaced.
    pub fn publish_terminal_failure(&self, record: &TerminalFailureRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.candidate
            .write_json_once(TERMINAL_FAILURE_FILE, record)
    }

    /// Publish the phase-9 cutover result.
    pub fn publish_cutover_result(&self, record: &CutoverResultRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        if record.result == CutoverResultStatus::Failed {
            self.publish_failure_for(
                record.binding.clone(),
                TerminalFailureReason::CutoverFailed,
                record.verified_at_unix,
            )?;
        }
        self.candidate.write_json_once(CUTOVER_RESULT_FILE, record)
    }

    /// Publish the guarded merge attempt.
    pub fn publish_merge_attempt(&self, record: &MergeAttemptRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        self.ensure_cutover_succeeded()?;
        if record.result != MergeAttemptStatus::Succeeded {
            self.publish_failure_for(
                record.binding.clone(),
                TerminalFailureReason::MergeMismatch,
                record.attempted_at_unix,
            )?;
        }
        self.candidate.write_json_once(MERGE_ATTEMPT_FILE, record)
    }

    /// Publish post-merge reconciliation. A non-exact tree is terminal.
    pub fn publish_post_merge_reconciliation(
        &self,
        record: &PostMergeReconciliationRecord,
    ) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        self.ensure_cutover_succeeded()?;
        self.ensure_merge_succeeded()?;
        if !record.exact_tree || record.expected_tree_oid != record.binding.tree_oid {
            self.publish_failure_for(
                record.binding.clone(),
                TerminalFailureReason::MergeMismatch,
                record.reconciled_at_unix,
            )?;
        }
        self.candidate
            .write_json_once(POST_MERGE_RECONCILIATION_FILE, record)
    }

    /// Publish the exact post-merge seal.
    pub fn publish_post_merge_seal(&self, record: &PostMergeSealRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        self.ensure_cutover_succeeded()?;
        self.ensure_merge_succeeded()?;
        if record.tree_oid != record.binding.tree_oid {
            return Err(DeliveryError::new(
                "post-merge seal tree does not match the approved candidate tree",
            ));
        }
        let reconciliation: PostMergeReconciliationRecord = self
            .candidate
            .read_json(POST_MERGE_RECONCILIATION_FILE)
            .map_err(|_| {
                DeliveryError::new(
                    "post-merge reconciliation is required before the post-merge seal",
                )
            })?;
        if !reconciliation.exact_tree
            || reconciliation.binding != record.binding
            || reconciliation.observed_tree_oid != record.tree_oid
        {
            return Err(DeliveryError::new(
                "post-merge seal requires the exact approved tree",
            ));
        }
        self.candidate.write_json_once(POST_MERGE_SEAL_FILE, record)
    }

    /// Publish the separately consented finalization record.
    pub fn publish_finalization(&self, record: &FinalizationRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        let seal: PostMergeSealRecord = self
            .candidate
            .read_json(POST_MERGE_SEAL_FILE)
            .map_err(|_| DeliveryError::new("post-merge seal is required before finalization"))?;
        if seal.binding != record.binding || seal.tree_oid != record.binding.tree_oid {
            return Err(DeliveryError::new(
                "finalization is not bound to the exact post-merge seal",
            ));
        }
        self.candidate.write_json_once(FINALIZATION_FILE, record)
    }

    /// Publish the final close record.
    pub fn publish_close(&self, record: &CloseRecord) -> Result<String> {
        record.validate().map_err(to_delivery_error)?;
        self.ensure_bound()?;
        self.ensure_no_terminal()?;
        let finalization: FinalizationRecord = self
            .candidate
            .read_json(FINALIZATION_FILE)
            .map_err(|_| DeliveryError::new("finalization is required before close"))?;
        if finalization.binding != record.binding {
            return Err(DeliveryError::new(
                "close is not bound to the finalized candidate",
            ));
        }
        self.candidate.write_json_once(CLOSE_FILE, record)
    }

    /// Return the durable state derived from write-once records.
    pub fn state(&self) -> Result<DurableDeliveryState> {
        if self.candidate.artifact_exists(TERMINAL_FAILURE_FILE)? {
            return Ok(DurableDeliveryState::Failed);
        }
        if self.candidate.artifact_exists(CLOSE_FILE)? {
            return Ok(DurableDeliveryState::Closed);
        }
        if self.candidate.artifact_exists(FINALIZATION_FILE)? {
            return Ok(DurableDeliveryState::Finalized);
        }
        if self.candidate.artifact_exists(POST_MERGE_SEAL_FILE)? {
            return Ok(DurableDeliveryState::PostMergeSealed);
        }
        if self.candidate.artifact_exists(MERGE_ATTEMPT_FILE)? {
            return Ok(DurableDeliveryState::MergeAttempted);
        }
        if self.candidate.artifact_exists(CUTOVER_RESULT_FILE)? {
            return Ok(DurableDeliveryState::CutoverSucceeded);
        }
        if self.candidate.artifact_exists(BINDING_REQUEST_FILE)? {
            return Ok(DurableDeliveryState::Bound);
        }
        Ok(DurableDeliveryState::Converging)
    }

    /// Admit a replacement candidate only while still converging.
    pub fn ensure_replacement_allowed(&self) -> Result<()> {
        for file in [
            BINDING_REQUEST_FILE,
            TERMINAL_FAILURE_FILE,
            CUTOVER_RESULT_FILE,
            MERGE_ATTEMPT_FILE,
            POST_MERGE_RECONCILIATION_FILE,
            POST_MERGE_SEAL_FILE,
            FINALIZATION_FILE,
            CLOSE_FILE,
        ] {
            if self.candidate.artifact_exists(file)? {
                return Err(DeliveryError::new(
                    "delivery candidate is already bound or terminal; no replacement is admitted",
                ));
            }
        }
        Ok(())
    }

    /// Publish a caller-classified post-binding terminal failure.
    pub fn fail_terminal(
        &self,
        binding: DeliveryBinding,
        reason: TerminalFailureReason,
        failed_at_unix: u64,
    ) -> Result<String> {
        self.ensure_bound()?;
        let failed_at = RecoveryUnixSeconds::new(failed_at_unix).map_err(to_delivery_error)?;
        self.publish_failure_for(binding, reason, failed_at)
    }

    /// Validate a bound recovery record at a later boundary. Any failure after
    /// binding publishes the one terminal failure before returning.
    pub fn validate_bound(
        &self,
        snapshot: &SnapshotView,
        attestation: &RecoveryAttestation,
        recovery_binding: &RecoveryBinding,
        verifier_now: u64,
        required_remaining_ttl: u64,
    ) -> Result<RecoveryValidation> {
        self.ensure_bound()?;
        let stored: BindingRequestRecord = self.candidate.read_json(BINDING_REQUEST_FILE)?;
        let current: SnapshotView = self.candidate.read_json(super::storage::SNAPSHOT_FILE)?;
        if current != *snapshot {
            let failed_at = RecoveryUnixSeconds::new(verifier_now).map_err(to_delivery_error)?;
            self.publish_failure_for(
                stored.binding,
                TerminalFailureReason::ContentDrift,
                failed_at,
            )?;
            return Err(DeliveryError::new(
                "bound candidate snapshot drifted and delivery is terminal",
            ));
        }
        let binding = match DeliveryBinding::from_snapshot(snapshot, attestation, recovery_binding)
        {
            Ok(binding) => binding,
            Err(error) => {
                let failed_at =
                    RecoveryUnixSeconds::new(verifier_now).map_err(to_delivery_error)?;
                self.publish_failure_for(
                    stored.binding,
                    TerminalFailureReason::EvidenceDrift,
                    failed_at,
                )?;
                return Err(to_delivery_error(error));
            }
        };
        if stored.binding != binding {
            let failed_at = RecoveryUnixSeconds::new(verifier_now).map_err(to_delivery_error)?;
            self.publish_failure_for(
                stored.binding,
                TerminalFailureReason::EvidenceDrift,
                failed_at,
            )?;
            return Err(DeliveryError::new(
                "bound delivery evidence drifted and is terminal",
            ));
        }
        match attestation.validate_at(recovery_binding, verifier_now, required_remaining_ttl) {
            Ok(validation) => Ok(validation),
            Err(error) => {
                let reason = if matches!(
                    error,
                    RecoveryError::Freshness
                        | RecoveryError::Expiry
                        | RecoveryError::InsufficientTtl
                ) {
                    TerminalFailureReason::Expired
                } else {
                    TerminalFailureReason::RecoveryInvalid
                };
                let failed_at =
                    RecoveryUnixSeconds::new(verifier_now).map_err(to_delivery_error)?;
                self.publish_failure_for(stored.binding, reason, failed_at)?;
                Err(DeliveryError::new(
                    "bound recovery evidence is invalid and delivery is terminal",
                ))
            }
        }
    }

    fn ensure_bound(&self) -> Result<()> {
        if self.candidate.artifact_exists(BINDING_REQUEST_FILE)? {
            Ok(())
        } else {
            Err(DeliveryError::new(
                "binding request is required before delivery state can advance",
            ))
        }
    }

    fn ensure_no_terminal(&self) -> Result<()> {
        if self.candidate.artifact_exists(TERMINAL_FAILURE_FILE)? {
            Err(DeliveryError::new(
                "delivery has one terminal failure and cannot advance",
            ))
        } else {
            Ok(())
        }
    }

    fn ensure_cutover_succeeded(&self) -> Result<()> {
        let record: CutoverResultRecord = self
            .candidate
            .read_json(CUTOVER_RESULT_FILE)
            .map_err(|_| DeliveryError::new("cutover result is required before merge"))?;
        if record.result == CutoverResultStatus::Succeeded {
            Ok(())
        } else {
            Err(DeliveryError::new(
                "a failed cutover result cannot continue to merge",
            ))
        }
    }

    fn ensure_merge_succeeded(&self) -> Result<()> {
        let record: MergeAttemptRecord = self
            .candidate
            .read_json(MERGE_ATTEMPT_FILE)
            .map_err(|_| DeliveryError::new("merge attempt is required before reconciliation"))?;
        if record.result == MergeAttemptStatus::Succeeded {
            Ok(())
        } else {
            Err(DeliveryError::new(
                "a non-successful merge attempt cannot continue",
            ))
        }
    }

    /// Validate the bound recovery point and the protected frozen closure
    /// before a host mutation boundary.
    pub fn validate_bound_with_closure(
        &self,
        snapshot: &SnapshotView,
        attestation: &RecoveryAttestation,
        recovery_binding: &RecoveryBinding,
        closure: &FrozenClosure,
        verifier_now: u64,
        required_remaining_ttl: u64,
    ) -> Result<RecoveryValidation> {
        self.ensure_bound()?;
        if let Err(error) = closure.validate_for(recovery_binding) {
            let binding = DeliveryBinding::from_snapshot(snapshot, attestation, recovery_binding)
                .map_err(to_delivery_error)?;
            let failed_at = RecoveryUnixSeconds::new(verifier_now).map_err(to_delivery_error)?;
            self.publish_failure_for(binding, TerminalFailureReason::RecoveryInvalid, failed_at)?;
            return Err(to_delivery_error(error));
        }
        self.validate_bound(
            snapshot,
            attestation,
            recovery_binding,
            verifier_now,
            required_remaining_ttl,
        )
    }

    fn publish_failure_for(
        &self,
        binding: DeliveryBinding,
        reason: TerminalFailureReason,
        at: RecoveryUnixSeconds,
    ) -> Result<String> {
        let record = TerminalFailureRecord {
            artifact_kind: TERMINAL_FAILURE_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            binding,
            reason,
            failed_at_unix: at,
        };
        record.validate().map_err(to_delivery_error)?;
        self.candidate
            .write_json_once(TERMINAL_FAILURE_FILE, &record)
    }
}

/// Delivery errors that never include raw recovery payloads, paths, IDs, or
/// operator data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// Canonical JSON rejected a duplicate, unknown numeric, or trailing value.
    CanonicalJson(CanonicalJsonError),
    /// Typed JSON decoding failed.
    Json,
    /// The record exceeded the bounded artifact size.
    TooLarge,
    /// Fixed artifact, version, or program shape failed.
    Shape,
    /// A digest or object identity failed.
    Binding,
    /// Qualification was incomplete.
    Qualification,
    /// Timestamp ordering or bounded checked arithmetic failed.
    Timestamp,
    /// Exact derived expiry failed.
    Expiry,
    /// Freshness failed at a sampled verifier time.
    Freshness,
    /// The caller-provided remaining lifetime is insufficient.
    InsufficientTtl,
    /// A closure or protected GC root is unavailable.
    ClosureUnavailable,
    /// The closure path, generation, or root does not match.
    ClosureMismatch,
    /// The system clock could not produce a bounded timestamp.
    Clock,
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalJson(_) => "strict recovery JSON rejected",
            Self::Json => "recovery attestation shape rejected",
            Self::TooLarge => "recovery attestation is too large",
            Self::Shape => "recovery attestation contract rejected",
            Self::Binding => "recovery attestation binding rejected",
            Self::Qualification => "recovery qualification rejected",
            Self::Timestamp => "recovery timestamp rejected",
            Self::Expiry => "recovery expiry rejected",
            Self::Freshness => "recovery evidence is stale or not yet valid",
            Self::InsufficientTtl => "recovery evidence lacks required remaining lifetime",
            Self::ClosureUnavailable => "pinned closure or protected GC root is unavailable",
            Self::ClosureMismatch => "pinned closure binding rejected",
            Self::Clock => "verifier clock rejected",
        })
    }
}

impl std::error::Error for RecoveryError {}

impl From<RecoveryError> for DeliveryError {
    fn from(error: RecoveryError) -> Self {
        DeliveryError::new(error.to_string())
    }
}

type RecoveryResult<T> = std::result::Result<T, RecoveryError>;

fn to_delivery_error(error: RecoveryError) -> DeliveryError {
    error.into()
}

fn validate_sha256_value(value: &str) -> RecoveryResult<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(RecoveryError::Binding)
    }
}

fn validate_bundle_generation(value: &str) -> RecoveryResult<()> {
    if value.is_empty()
        || value.len() > 128
        || value.bytes().any(|byte| {
            byte.is_ascii_control() || byte.is_ascii_whitespace() || matches!(byte, b'/' | b'\\')
        })
    {
        Err(RecoveryError::Binding)
    } else {
        Ok(())
    }
}

fn validate_record_header(
    found: &str,
    expected: &str,
    schema_version: u32,
    binding: &DeliveryBinding,
) -> RecoveryResult<()> {
    if found != expected
        || schema_version != DELIVERY_SCHEMA_VERSION
        || binding.program.is_empty()
        || binding.wave.is_empty()
        || binding.bundle_generation.is_empty()
    {
        return Err(RecoveryError::Shape);
    }
    validate_bundle_generation(&binding.bundle_generation)
}

fn domain_digest(domain: &str, bytes: &[u8]) -> Sha256Digest {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    let result = digest.finalize();
    let mut rendered = String::with_capacity(64);
    for byte in result {
        use std::fmt::Write as _;
        let _ = write!(rendered, "{byte:02x}");
    }
    Sha256Digest(rendered)
}

/// Digest an opaque host identity without retaining the identity.
pub fn digest_host_identity(machine_id: &str) -> Sha256Digest {
    domain_digest(
        RECOVERY_HOST_DOMAIN,
        machine_id.to_ascii_lowercase().as_bytes(),
    )
}

/// Digest an operator's numeric peer subject without retaining the UID.
pub fn digest_operator_subject(uid: u32) -> Sha256Digest {
    domain_digest(RECOVERY_OPERATOR_DOMAIN, uid.to_string().as_bytes())
}

/// Digest an opaque external locator without retaining the locator.
pub fn digest_recovery_locator(locator: &str) -> Sha256Digest {
    domain_digest(RECOVERY_LOCATOR_DOMAIN, locator.as_bytes())
}

/// Digest exact restore-instruction bytes without retaining their contents.
pub fn digest_restore_instructions(bytes: &[u8]) -> Sha256Digest {
    domain_digest(RECOVERY_RESTORE_DOMAIN, bytes)
}

/// Digest the exact UTF-8 spelling of a pinned closure store path.
pub fn digest_store_path(path: &Path) -> RecoveryResult<Sha256Digest> {
    let path = path.to_str().ok_or(RecoveryError::Binding)?;
    Ok(domain_digest(CLOSURE_STORE_PATH_DOMAIN, path.as_bytes()))
}

fn sampled_unix_seconds() -> RecoveryResult<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RecoveryError::Clock)?;
    RecoveryUnixSeconds::new(now.as_secs()).map(|value| value.as_u64())
}

fn read_attestation(path: &Path) -> RecoveryResult<Vec<u8>> {
    let mut file = fs::File::open(path).map_err(|_| RecoveryError::Json)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RecoveryError::Json)?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(RecoveryError::TooLarge);
    }
    Ok(bytes)
}

/// Request for the recovery-import delivery stage.
pub struct RecoveryImportRequest {
    snapshot_path: PathBuf,
    attestation_path: PathBuf,
    checkouts: std::collections::BTreeMap<String, PathBuf>,
    state_dir: Option<PathBuf>,
    binding: RecoveryBinding,
    required_remaining_ttl: u64,
    verifier_now: u64,
    command: String,
}

impl RecoveryImportRequest {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = CliOptions::parse(args)?;
        let snapshot_path = options.required_path("--snapshot")?;
        let attestation_path = options.required_path("--attestation")?;
        let checkouts = options.repository_roots()?;
        let candidate_id = CandidateId::parse(options.required_string("--candidate-id")?)?;
        let commit_oid = options.required_string("--commit-oid")?;
        let tree_oid = options.required_string("--tree-oid")?;
        let closure = Sha256Digest::parse(options.required_string("--closure-store-path-sha256")?)
            .map_err(to_delivery_error)?;
        let bundle_generation = options.required_string("--bundle-generation")?;
        let preview = Sha256Digest::parse(options.required_string("--preview-sha256")?)
            .map_err(to_delivery_error)?;
        let host = Sha256Digest::parse(options.required_string("--host-identity-sha256")?)
            .map_err(to_delivery_error)?;
        let operator = Sha256Digest::parse(options.required_string("--operator-subject-sha256")?)
            .map_err(to_delivery_error)?;
        let restore =
            Sha256Digest::parse(options.required_string("--restore-instructions-sha256")?)
                .map_err(to_delivery_error)?;
        let locator =
            Sha256Digest::parse(options.required_string("--recovery-point-locator-sha256")?)
                .map_err(to_delivery_error)?;
        let required_remaining_ttl = options
            .required_string("--required-remaining-ttl-seconds")?
            .parse::<u64>()
            .map_err(|_| DeliveryError::usage("required remaining TTL must be an integer"))?;
        let verifier_now = options
            .required_string("--verifier-now-unix")?
            .parse::<u64>()
            .map_err(|_| DeliveryError::usage("verifier time must be an integer"))?;
        let command = options.required_string("--command")?;
        let state_dir = options.optional_path("--state-dir")?;
        options.finish()?;
        validate_identifier(&command, "recovery verifier command")?;
        let binding = RecoveryBinding::new(
            candidate_id,
            commit_oid,
            tree_oid,
            closure,
            bundle_generation,
            preview,
            host,
            operator,
            restore,
            locator,
        )
        .map_err(to_delivery_error)?;
        Ok(Self {
            snapshot_path,
            attestation_path,
            checkouts,
            state_dir,
            binding,
            required_remaining_ttl,
            verifier_now,
            command,
        })
    }
}

/// Import one strict recovery attestation into the existing candidate evidence
/// layout. The locator and raw attestation input never enter the candidate.
pub fn run(args: &[String]) -> Result<WorkflowOutput> {
    let request = RecoveryImportRequest::parse(args)?;
    let state = StateRoot::prepare(
        &request.checkouts.values().cloned().collect::<Vec<_>>(),
        request.state_dir.as_deref(),
    )?;
    let snapshot_path = state.resolve_artifact_ref(&request.snapshot_path);
    let (candidate, snapshot) = super::open_candidate(&state, &snapshot_path)?;
    let bytes = read_attestation(&request.attestation_path).map_err(to_delivery_error)?;
    import_attestation(
        &candidate,
        &snapshot,
        &bytes,
        &request.binding,
        request.verifier_now,
        request.required_remaining_ttl,
        &request.command,
    )
}

/// Import one recovery attestation through the existing evidence writer.
pub fn import_attestation(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    bytes: &[u8],
    binding: &RecoveryBinding,
    verifier_now: u64,
    required_remaining_ttl: u64,
    command: &str,
) -> Result<WorkflowOutput> {
    snapshot.validate(candidate)?;
    let current: SnapshotView = candidate.read_json(super::storage::SNAPSHOT_FILE)?;
    if current != *snapshot {
        return Err(DeliveryError::new(
            "candidate snapshot changed before recovery import",
        ));
    }
    let attestation = RecoveryAttestation::decode_json(bytes).map_err(to_delivery_error)?;
    if attestation.candidate_id != snapshot.candidate_id
        || binding.candidate_id != snapshot.candidate_id
    {
        return Err(DeliveryError::new(
            "recovery evidence is not bound to the candidate snapshot",
        ));
    }
    validate_identifier(command, "recovery verifier command")?;
    let validation = attestation
        .validate_at(binding, verifier_now, required_remaining_ttl)
        .map_err(to_delivery_error)?;
    let canonical = attestation.canonical_bytes().map_err(to_delivery_error)?;
    let record = EvidenceRecord {
        artifact_kind: super::model::EVIDENCE_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: snapshot.program().to_owned(),
        wave: snapshot.wave().to_owned(),
        candidate_id: snapshot.candidate_id.clone(),
        content_id: snapshot.content_id.clone(),
        snapshot_sha256: snapshot.snapshot_sha256.clone(),
        lane: EvidenceLane::LocalHost,
        validation: RECOVERY_EVIDENCE_VALIDATION.to_owned(),
        result: super::model::EvidenceResult::Passed,
        imported_at_unix: verifier_now,
        command: Some(command.to_owned()),
        output: Some(OutputDigest {
            sha256: validation.attestation_sha256().as_str().to_owned(),
            bytes: canonical.len() as u64,
        }),
        locator: Some(
            attestation
                .recovery_point_locator_sha256
                .as_str()
                .to_owned(),
        ),
    };
    record.validate()?;
    let path = evidence::import_recovery(candidate, &record)?;
    WorkflowOutput::ok(WaveCommand::RecoveryImport)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &[u8] = br#"{"artifactKind":"d2b-recovery-point-attestation","schemaVersion":1,"program":"ADR046","candidateId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commitOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","treeOid":"cccccccccccccccccccccccccccccccccccccccc","closureStorePathSha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","bundleGeneration":"generation-1","hostIdentitySha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","operatorSubjectSha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","previewSha256":"1111111111111111111111111111111111111111111111111111111111111111","recoveryPointKind":"full-host-snapshot","recoveryPointLocatorSha256":"2222222222222222222222222222222222222222222222222222222222222222","restoreInstructionsSha256":"3333333333333333333333333333333333333333333333333333333333333333","previewedAtUnix":1000,"capturedAtUnix":1001,"verifiedAtUnix":1002,"attestedAtUnix":1003,"retentionUntilUnix":90000,"expiresAtUnix":87401,"verificationMethod":"snapshot-readback","verificationResult":"passed","qualification":{"bootAndSystemStateCovered":true,"affectedArtifactInventoryCovered":true,"preservedIdentityStateCovered":true,"sameHostRestoreTarget":true,"readOnlyUntilExpiry":true},"result":"passed"}"#;

    #[test]
    fn canonical_valid_attestation_is_accepted() {
        let attestation = RecoveryAttestation::decode_json(VALID).expect("valid attestation");
        let binding = RecoveryBinding::test_fixture();
        let result = attestation
            .validate_at(&binding, 2_000, 0)
            .expect("attestation validates");
        assert_eq!(result.expires_at().as_u64(), 87_401);
    }

    #[test]
    fn every_timestamp_and_binding_is_checked_independently() {
        let binding = RecoveryBinding::test_fixture();
        for (field, replacement) in [
            (
                "candidateId",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            ("commitOid", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("treeOid", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            (
                "closureStorePathSha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            ("bundleGeneration", "generation-2"),
            (
                "previewSha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            (
                "hostIdentitySha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            (
                "operatorSubjectSha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            (
                "restoreInstructionsSha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            (
                "recoveryPointLocatorSha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ] {
            let changed = replace_json_string(VALID, field, replacement);
            let attestation = RecoveryAttestation::decode_json(&changed).expect("shape");
            assert!(
                attestation.validate_at(&binding, 2_000, 0).is_err(),
                "{field} drift must fail"
            );
        }

        for (field, value) in [
            ("previewedAtUnix", 1004),
            ("capturedAtUnix", 999),
            ("verifiedAtUnix", 1000),
            ("attestedAtUnix", 2001),
            ("retentionUntilUnix", 87_400),
            ("expiresAtUnix", 87_400),
        ] {
            let changed = replace_json_integer(VALID, field, value);
            if let Ok(attestation) = RecoveryAttestation::decode_json(&changed) {
                assert!(
                    attestation.validate_at(&binding, 2_000, 0).is_err(),
                    "{field} drift must fail"
                );
            }
        }
    }

    #[test]
    fn malformed_canonical_inputs_fail_closed() {
        let duplicate = VALID
            .strip_suffix(b"}")
            .expect("object")
            .iter()
            .copied()
            .chain(br#","result":"passed"}"#.iter().copied())
            .collect::<Vec<_>>();
        assert!(RecoveryAttestation::decode_json(&duplicate).is_err());

        for malformed in [
            &br#"{"artifactKind":"d2b-recovery-point-attestation","schemaVersion":1,"program":"d2b-3.0","candidateId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commitOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","treeOid":"cccccccccccccccccccccccccccccccccccccccc","closureStorePathSha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","bundleGeneration":"generation-1","hostIdentitySha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","operatorSubjectSha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","previewSha256":"1111111111111111111111111111111111111111111111111111111111111111","recoveryPointKind":"full-host-snapshot","recoveryPointLocatorSha256":"2222222222222222222222222222222222222222222222222222222222222222","restoreInstructionsSha256":"3333333333333333333333333333333333333333333333333333333333333333","previewedAtUnix":1000.0,"capturedAtUnix":1001,"verifiedAtUnix":1002,"attestedAtUnix":1003,"retentionUntilUnix":90000,"expiresAtUnix":87401,"verificationMethod":"snapshot-readback","verificationResult":"passed","qualification":{"bootAndSystemStateCovered":true,"affectedArtifactInventoryCovered":true,"preservedIdentityStateCovered":true,"sameHostRestoreTarget":true,"readOnlyUntilExpiry":true},"result":"passed"}"#[..],
            &br#"{"artifactKind":"d2b-recovery-point-attestation","schemaVersion":1,"program":"d2b-3.0","candidateId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commitOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","treeOid":"cccccccccccccccccccccccccccccccccccccccc","closureStorePathSha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","bundleGeneration":"generation-1","hostIdentitySha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","operatorSubjectSha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","previewSha256":"1111111111111111111111111111111111111111111111111111111111111111","recoveryPointKind":"full-host-snapshot","recoveryPointLocatorSha256":"2222222222222222222222222222222222222222222222222222222222222222","restoreInstructionsSha256":"3333333333333333333333333333333333333333333333333333333333333333","previewedAtUnix":1000,"capturedAtUnix":1001,"verifiedAtUnix":1002,"attestedAtUnix":1003,"retentionUntilUnix":90000,"expiresAtUnix":87401,"verificationMethod":"snapshot-readback","verificationResult":"passed","qualification":{"bootAndSystemStateCovered":true,"affectedArtifactInventoryCovered":true,"preservedIdentityStateCovered":true,"sameHostRestoreTarget":true,"readOnlyUntilExpiry":true},"result":"passed","extra":1}"#[..],
            &br#"{"artifactKind":"d2b-recovery-point-attestation","schemaVersion":1,"program":"d2b-3.0","candidateId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commitOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","treeOid":"cccccccccccccccccccccccccccccccccccccccc","closureStorePathSha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","bundleGeneration":"generation-1","hostIdentitySha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","operatorSubjectSha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","previewSha256":"1111111111111111111111111111111111111111111111111111111111111111","recoveryPointKind":"full-host-snapshot","recoveryPointLocatorSha256":"2222222222222222222222222222222222222222222222222222222222222222","restoreInstructionsSha256":"3333333333333333333333333333333333333333333333333333333333333333","previewedAtUnix":1000,"capturedAtUnix":1001,"verifiedAtUnix":1002,"attestedAtUnix":1003,"retentionUntilUnix":90000,"expiresAtUnix":87401,"verificationMethod":"snapshot-readback","verificationResult":"passed","qualification":{"bootAndSystemStateCovered":true,"affectedArtifactInventoryCovered":true,"preservedIdentityStateCovered":true,"sameHostRestoreTarget":true,"readOnlyUntilExpiry":true},"result":"passed"} trailing"#[..],
        ] {
            assert!(RecoveryAttestation::decode_json(malformed).is_err());
        }
    }

    #[test]
    fn every_required_field_and_qualification_member_is_required_and_typed() {
        let fields = [
            "artifactKind",
            "schemaVersion",
            "program",
            "candidateId",
            "commitOid",
            "treeOid",
            "closureStorePathSha256",
            "bundleGeneration",
            "hostIdentitySha256",
            "operatorSubjectSha256",
            "previewSha256",
            "recoveryPointKind",
            "recoveryPointLocatorSha256",
            "restoreInstructionsSha256",
            "previewedAtUnix",
            "capturedAtUnix",
            "verifiedAtUnix",
            "attestedAtUnix",
            "retentionUntilUnix",
            "expiresAtUnix",
            "verificationMethod",
            "verificationResult",
            "qualification",
            "result",
        ];
        for field in fields {
            let mut value: serde_json::Value =
                serde_json::from_slice(VALID).expect("valid fixture");
            value.as_object_mut().expect("object").remove(field);
            let bytes = serde_json::to_vec(&value).expect("missing field");
            assert!(
                RecoveryAttestation::decode_json(&bytes).is_err(),
                "missing {field} must fail"
            );
        }

        let type_changes = [
            ("artifactKind", serde_json::json!(1)),
            ("schemaVersion", serde_json::json!("1")),
            ("program", serde_json::json!(1)),
            ("candidateId", serde_json::json!(1)),
            ("commitOid", serde_json::json!(1)),
            ("treeOid", serde_json::json!(1)),
            ("closureStorePathSha256", serde_json::json!(1)),
            ("bundleGeneration", serde_json::json!(1)),
            ("hostIdentitySha256", serde_json::json!(1)),
            ("operatorSubjectSha256", serde_json::json!(1)),
            ("previewSha256", serde_json::json!(1)),
            ("recoveryPointKind", serde_json::json!(1)),
            ("recoveryPointLocatorSha256", serde_json::json!(1)),
            ("restoreInstructionsSha256", serde_json::json!(1)),
            ("previewedAtUnix", serde_json::json!("1000")),
            ("capturedAtUnix", serde_json::json!("1001")),
            ("verifiedAtUnix", serde_json::json!("1002")),
            ("attestedAtUnix", serde_json::json!("1003")),
            ("retentionUntilUnix", serde_json::json!("90000")),
            ("expiresAtUnix", serde_json::json!("87401")),
            ("verificationMethod", serde_json::json!(1)),
            ("verificationResult", serde_json::json!(1)),
            ("qualification", serde_json::json!("complete")),
            ("result", serde_json::json!(1)),
        ];
        for (field, replacement) in type_changes {
            let mut value: serde_json::Value =
                serde_json::from_slice(VALID).expect("valid fixture");
            value
                .as_object_mut()
                .expect("object")
                .insert(field.to_owned(), replacement);
            let bytes = serde_json::to_vec(&value).expect("type change");
            assert!(
                RecoveryAttestation::decode_json(&bytes).is_err(),
                "wrong type for {field} must fail"
            );
        }

        for field in [
            "bootAndSystemStateCovered",
            "affectedArtifactInventoryCovered",
            "preservedIdentityStateCovered",
            "sameHostRestoreTarget",
            "readOnlyUntilExpiry",
        ] {
            let mut value: serde_json::Value =
                serde_json::from_slice(VALID).expect("valid fixture");
            value["qualification"]
                .as_object_mut()
                .expect("qualification")
                .remove(field);
            let bytes = serde_json::to_vec(&value).expect("qualification");
            assert!(
                RecoveryAttestation::decode_json(&bytes).is_err(),
                "missing qualification member {field} must fail"
            );

            let mut value: serde_json::Value =
                serde_json::from_slice(VALID).expect("valid fixture");
            value["qualification"][field] = serde_json::json!(false);
            let bytes = serde_json::to_vec(&value).expect("qualification");
            assert!(
                RecoveryAttestation::decode_json(&bytes).is_err(),
                "false qualification member {field} must fail"
            );
        }

        let mut extra: serde_json::Value = serde_json::from_slice(VALID).expect("valid fixture");
        extra["qualification"]["extra"] = serde_json::json!(true);
        assert!(RecoveryAttestation::decode_json(&serde_json::to_vec(&extra).unwrap()).is_err());
    }

    #[test]
    fn verifier_clock_is_sampled_once_and_ttl_is_explicit() {
        let attestation = RecoveryAttestation::decode_json(VALID).expect("valid attestation");
        let binding = RecoveryBinding::test_fixture();
        let mut samples = 0;
        attestation
            .validate_with_clock(&binding, 10, || {
                samples += 1;
                2_000
            })
            .expect("clock validation");
        assert_eq!(samples, 1);
        assert!(attestation.validate_at(&binding, 2_000, 87_402).is_err());
        attestation
            .validate_at(&binding, 2_000, 87_401 - 2_000)
            .expect("exact required remaining TTL");
    }

    fn delivery_fixture() -> (
        crate::delivery::storage::tests::Scratch,
        CandidateDir,
        SnapshotView,
        RecoveryAttestation,
        RecoveryBinding,
        DeliveryBinding,
    ) {
        let scratch = crate::delivery::storage::tests::Scratch::new("recovery-delivery");
        let (_state, candidate, snapshot) =
            crate::delivery::test_support::candidate_with_snapshot(&scratch);
        let mut attestation = RecoveryAttestation::decode_json(VALID).expect("attestation");
        attestation.candidate_id = snapshot.candidate_id.clone();
        let recovery_binding = RecoveryBinding::new(
            snapshot.candidate_id.clone(),
            attestation.commit_oid.as_str(),
            attestation.tree_oid.as_str(),
            attestation.closure_store_path_sha256.clone(),
            attestation.bundle_generation.clone(),
            attestation.preview_sha256.clone(),
            attestation.host_identity_sha256.clone(),
            attestation.operator_subject_sha256.clone(),
            attestation.restore_instructions_sha256.clone(),
            attestation.recovery_point_locator_sha256.clone(),
        )
        .expect("recovery binding");
        let delivery_binding =
            DeliveryBinding::from_snapshot(&snapshot, &attestation, &recovery_binding)
                .expect("delivery binding");
        (
            scratch,
            candidate,
            snapshot,
            attestation,
            recovery_binding,
            delivery_binding,
        )
    }

    #[test]
    fn delivery_can_converge_before_binding_but_terminal_failure_blocks_replacement() {
        let (_scratch, candidate, snapshot, attestation, recovery, binding) = delivery_fixture();
        let ledger = DeliveryLedger::new(&candidate);
        ledger
            .ensure_replacement_allowed()
            .expect("prebinding convergence remains open");
        ledger
            .bind(&snapshot, &attestation, &recovery, 2_000, 0)
            .expect("bind");
        assert_eq!(ledger.state().expect("state"), DurableDeliveryState::Bound);
        assert!(
            candidate
                .write_json(super::super::storage::SNAPSHOT_FILE, &snapshot)
                .is_err(),
            "a bound candidate snapshot cannot be replaced"
        );
        assert!(ledger.ensure_replacement_allowed().is_err());
        ledger
            .publish_terminal_failure(
                &TerminalFailureRecord::new(
                    binding.clone(),
                    TerminalFailureReason::ContentDrift,
                    2_001,
                )
                .expect("terminal"),
            )
            .expect("terminal failure");
        assert_eq!(ledger.state().expect("state"), DurableDeliveryState::Failed);
        assert!(
            ledger
                .publish_terminal_failure(
                    &TerminalFailureRecord::new(binding, TerminalFailureReason::Expired, 2_002)
                        .expect("second terminal"),
                )
                .is_err(),
            "the terminal record is write-once"
        );
    }

    #[test]
    fn prebinding_snapshot_refresh_remains_allowed() {
        let (_scratch, candidate, snapshot, _attestation, _recovery, _binding) = delivery_fixture();
        let refreshed = crate::delivery::test_support::rebased(&snapshot);
        candidate
            .write_json(crate::delivery::storage::SNAPSHOT_FILE, &refreshed)
            .expect("refresh before binding");
        let stored: SnapshotView = candidate
            .read_json(crate::delivery::storage::SNAPSHOT_FILE)
            .expect("snapshot");
        assert_eq!(stored.snapshot_sha256, refreshed.snapshot_sha256);
    }

    #[test]
    fn merge_mismatch_is_terminal_and_exact_tree_allows_post_merge_seal() {
        let (_scratch, candidate, _snapshot, _attestation, _recovery, binding) = delivery_fixture();
        let ledger = DeliveryLedger::new(&candidate);
        ledger
            .publish_binding_request(
                &BindingRequestRecord::new(binding.clone(), 0, 2_000).expect("binding"),
            )
            .expect("bind");
        ledger
            .publish_cutover_result(
                &CutoverResultRecord::new(binding.clone(), CutoverResultStatus::Succeeded, 2_001)
                    .expect("cutover"),
            )
            .expect("cutover result");
        ledger
            .publish_merge_attempt(
                &MergeAttemptRecord::new(
                    binding.clone(),
                    Sha256Digest::parse("4".repeat(64)).expect("expected"),
                    Sha256Digest::parse("4".repeat(64)).expect("observed"),
                    MergeAttemptStatus::Succeeded,
                    2_002,
                )
                .expect("merge"),
            )
            .expect("merge attempt");
        let tree = "c".repeat(40);
        ledger
            .publish_post_merge_reconciliation(
                &PostMergeReconciliationRecord::new(binding.clone(), &tree, &tree, 2_003)
                    .expect("reconciliation"),
            )
            .expect("reconciliation");
        ledger
            .publish_post_merge_seal(
                &PostMergeSealRecord::new(binding.clone(), &tree, 2_004).expect("seal"),
            )
            .expect("post-merge seal");
        assert_eq!(
            ledger.state().expect("state"),
            DurableDeliveryState::PostMergeSealed
        );
    }

    #[test]
    fn finalization_and_close_follow_the_exact_post_merge_seal() {
        let (_scratch, candidate, _snapshot, _attestation, _recovery, binding) = delivery_fixture();
        let ledger = DeliveryLedger::new(&candidate);
        ledger
            .publish_binding_request(
                &BindingRequestRecord::new(binding.clone(), 0, 2_000).expect("binding"),
            )
            .expect("bind");
        ledger
            .publish_cutover_result(
                &CutoverResultRecord::new(binding.clone(), CutoverResultStatus::Succeeded, 2_001)
                    .expect("cutover"),
            )
            .expect("cutover");
        ledger
            .publish_merge_attempt(
                &MergeAttemptRecord::new(
                    binding.clone(),
                    Sha256Digest::parse("4".repeat(64)).unwrap(),
                    Sha256Digest::parse("4".repeat(64)).unwrap(),
                    MergeAttemptStatus::Succeeded,
                    2_002,
                )
                .expect("merge"),
            )
            .expect("merge");
        let tree = "c".repeat(40);
        ledger
            .publish_post_merge_reconciliation(
                &PostMergeReconciliationRecord::new(binding.clone(), &tree, &tree, 2_003)
                    .expect("reconciliation"),
            )
            .expect("reconciliation");
        ledger
            .publish_post_merge_seal(
                &PostMergeSealRecord::new(binding.clone(), &tree, 2_004).expect("seal"),
            )
            .expect("seal");
        let consent = Sha256Digest::parse("5".repeat(64)).unwrap();
        ledger
            .publish_finalization(
                &FinalizationRecord::new(binding.clone(), consent, 2_005).expect("finalization"),
            )
            .expect("finalization");
        assert_eq!(
            ledger.state().expect("state"),
            DurableDeliveryState::Finalized
        );
        let close = CloseRecord::new(
            binding.clone(),
            Sha256Digest::parse("6".repeat(64)).unwrap(),
            2_006,
        )
        .expect("close");
        ledger.publish_close(&close).expect("close");
        assert_eq!(ledger.state().expect("state"), DurableDeliveryState::Closed);
        assert!(ledger.publish_close(&close).is_err(), "close is write-once");
    }

    #[test]
    fn post_binding_expiry_publishes_one_terminal_failure() {
        let (_scratch, candidate, snapshot, attestation, recovery, binding) = delivery_fixture();
        let ledger = DeliveryLedger::new(&candidate);
        ledger
            .publish_binding_request(
                &BindingRequestRecord::new(binding, 0, 2_000).expect("binding"),
            )
            .expect("bind");
        assert!(
            ledger
                .validate_bound(&snapshot, &attestation, &recovery, 87_401, 0)
                .is_err()
        );
        assert_eq!(ledger.state().expect("state"), DurableDeliveryState::Failed);
    }

    #[test]
    fn locator_and_closure_inputs_are_digest_only_and_closure_is_pinned() {
        let locator = "https://operator.example/recovery/secret";
        let digest = digest_recovery_locator(locator);
        let attestation = RecoveryAttestation::decode_json(VALID).expect("attestation");
        let rendered = serde_json::to_vec(&attestation).expect("record");
        assert!(!String::from_utf8_lossy(&rendered).contains(locator));
        assert!(!format!("{attestation:?}").contains(locator));
        assert_eq!(digest.as_str().len(), 64);

        let scratch = crate::delivery::storage::tests::Scratch::new("closure");
        let store = scratch.path.join("closure");
        fs::create_dir(&store).expect("store");
        let gcroot = scratch.path.join("gcroot");
        std::os::unix::fs::symlink(&store, &gcroot).expect("gcroot");
        let store_digest = digest_store_path(&store).expect("store digest");
        let closure = FrozenClosure::new(&store, &gcroot, store_digest.clone(), "generation-1")
            .expect("closure");
        closure.validate().expect("closure is pinned");
        let wrong = FrozenClosure::new(
            &store,
            &gcroot,
            Sha256Digest::parse("0".repeat(64)).unwrap(),
            "generation-1",
        )
        .expect("wrong closure");
        assert!(matches!(
            wrong.validate(),
            Err(RecoveryError::ClosureMismatch)
        ));
        fs::remove_file(&gcroot).expect("remove gcroot");
        assert!(matches!(
            closure.validate(),
            Err(RecoveryError::ClosureUnavailable)
        ));
    }

    #[test]
    fn recovery_import_uses_existing_evidence_and_is_write_once() {
        let (_scratch, candidate, snapshot, mut attestation, recovery, _binding) =
            delivery_fixture();
        attestation.candidate_id = snapshot.candidate_id.clone();
        let bytes = attestation
            .canonical_bytes()
            .expect("canonical attestation");
        let output = import_attestation(
            &candidate,
            &snapshot,
            &bytes,
            &recovery,
            2_000,
            0,
            "d2b-recovery-verify",
        )
        .expect("recovery import");
        assert_eq!(output.operation.as_str(), "recovery-import");
        let records = evidence::read_lane_records(
            &candidate,
            &snapshot.candidate_id,
            &snapshot.content_id,
            &snapshot.snapshot_sha256,
        )
        .expect("evidence");
        let record = records
            .iter()
            .find(|record| record.validation == RECOVERY_EVIDENCE_VALIDATION)
            .expect("recovery record");
        assert_eq!(
            record.locator.as_deref(),
            Some(attestation.recovery_point_locator_sha256.as_str())
        );
        let record_bytes = serde_json::to_vec(record).expect("record bytes");
        assert!(!String::from_utf8_lossy(&record_bytes).contains("https://"));
        assert!(
            import_attestation(
                &candidate,
                &snapshot,
                &bytes,
                &recovery,
                2_000,
                0,
                "d2b-recovery-verify",
            )
            .is_err(),
            "recovery evidence is no-replace"
        );
    }

    #[test]
    fn post_merge_tree_mismatch_is_terminal_and_cannot_be_sealed() {
        let (_scratch, candidate, _snapshot, _attestation, _recovery, binding) = delivery_fixture();
        let ledger = DeliveryLedger::new(&candidate);
        ledger
            .publish_binding_request(
                &BindingRequestRecord::new(binding.clone(), 0, 2_000).expect("binding"),
            )
            .expect("bind");
        ledger
            .publish_cutover_result(
                &CutoverResultRecord::new(binding.clone(), CutoverResultStatus::Succeeded, 2_001)
                    .expect("cutover"),
            )
            .expect("cutover");
        ledger
            .publish_merge_attempt(
                &MergeAttemptRecord::new(
                    binding.clone(),
                    Sha256Digest::parse("4".repeat(64)).unwrap(),
                    Sha256Digest::parse("4".repeat(64)).unwrap(),
                    MergeAttemptStatus::Succeeded,
                    2_002,
                )
                .expect("merge"),
            )
            .expect("merge");
        let mismatch = PostMergeReconciliationRecord::new(
            binding.clone(),
            "c".repeat(40),
            "d".repeat(40),
            2_003,
        )
        .expect("mismatch");
        ledger
            .publish_post_merge_reconciliation(&mismatch)
            .expect("durable mismatch");
        assert_eq!(ledger.state().expect("state"), DurableDeliveryState::Failed);
        assert!(
            ledger
                .publish_post_merge_seal(
                    &PostMergeSealRecord::new(binding, "c".repeat(40), 2_004).unwrap()
                )
                .is_err()
        );
    }

    fn replace_json_string(input: &[u8], key: &str, replacement: &str) -> Vec<u8> {
        let marker = format!("\"{key}\":\"");
        let text = std::str::from_utf8(input).expect("fixture utf8");
        let start = text.find(&marker).expect("key");
        let value_start = start + marker.len();
        let value_end = text[value_start..].find('"').expect("value") + value_start;
        format!(
            "{}{}{}",
            &text[..value_start],
            replacement,
            &text[value_end..]
        )
        .into_bytes()
    }

    fn replace_json_integer(input: &[u8], key: &str, replacement: u64) -> Vec<u8> {
        let marker = format!("\"{key}\":");
        let text = std::str::from_utf8(input).expect("fixture utf8");
        let start = text.find(&marker).expect("key");
        let value_start = start + marker.len();
        let value_end = text[value_start..]
            .find(|character: char| !character.is_ascii_digit())
            .expect("value")
            + value_start;
        format!(
            "{}{}{}",
            &text[..value_start],
            replacement,
            &text[value_end..]
        )
        .into_bytes()
    }
}
