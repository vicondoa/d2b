//! Typed audit schema for realm secrets lifecycle (provision / rotate /
//! rollback / retire) operations.
//!
//! This module mirrors the existing [`crate::ops::store_sync_audit`]
//! pattern: it is the typed shape of a broker audit record's
//! `operation_fields` object plus invariant-enforcing constructors and a
//! [`SecretsLifecycleAuditFields::validate`] pass, kept independent of
//! [`crate::ops::audit_op`] so the integrator can add exactly one new
//! `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
//! variant there (and a matching `from_operation_value` arm) in a
//! follow-up commit without this component editing that shared sink.
//!
//! Redaction contract: this is the **host-confidential** audit record
//! (broker audit log is `0640 root:d2bd`, per
//! `docs/reference/cgroup-delegation.md` § "Audit records"). It
//! deliberately never carries raw secret material, a raw filesystem
//! path, or a raw channel-binding/credential byte string — only a
//! FNV1a-64 `base_dir_hash` (parity with [`crate::ops::hosts::stable_hash_str`]
//! used by every other path-bearing broker audit record), a SHA-256
//! `material_digest_hex` fingerprint (so operators can correlate a
//! rotation to specific delivered material without ever seeing it),
//! and closed-set result/marker enums.

use serde::{Deserialize, Serialize};

/// Schema version for the secrets-lifecycle terminal audit record.
pub const SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION: u32 = 1;

/// Bound on `retained_generations` so a single audit record can never
/// grow unbounded (the operational retention policy in
/// `secrets_lifecycle.rs` keeps at most one retained generation, but
/// the audit schema allows a little headroom for a future policy
/// change without a schema bump).
pub const MAX_AUDITED_RETAINED_GENERATIONS: usize = 8;

/// Closed set of per-realm secret material kinds this component's
/// lifecycle covers. Matches the W8 `secrets-lifecycle` component scope:
/// TPM-bound credentials, guest signing keys, and security-key channel
/// state. Adding a new kind is a schema-version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// Rotation/retirement bookkeeping layered atop the swtpm-owned
    /// NVRAM identity (`swtpm_dir.rs` remains the sole owner of the
    /// physical TPM state dir; this kind never mutates it directly).
    TpmBoundCredential,
    /// Per-VM guest signing key material (e.g. an SSH host key
    /// generation lineage) tracked through provision/rotate/retire.
    GuestSigningKey,
    /// Security-key CTAPHID channel state (`channel_binding` +
    /// `reconnect_generation`) consumed by the guest
    /// `d2b-sk-frontend` `ComponentSession` policy.
    SecurityKeyChannelState,
}

impl SecretKind {
    /// Stable, path-safe slug used as the on-disk directory component
    /// and as the audit-record `kind` discriminant's string form for
    /// any downstream consumer that only speaks JSON.
    pub fn as_slug(self) -> &'static str {
        match self {
            SecretKind::TpmBoundCredential => "tpm-bound-credential",
            SecretKind::GuestSigningKey => "guest-signing-key",
            SecretKind::SecurityKeyChannelState => "security-key-channel-state",
        }
    }
}

/// The lifecycle action a single audit record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    Provision,
    Rotate,
    Rollback,
    Retire,
}

/// Terminal disposition of a lifecycle action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleResult {
    /// Fresh generation 1 material was provisioned.
    Created,
    /// A new generation was created and `current` was atomically
    /// swapped to it.
    Rotated,
    /// `current` was atomically swapped back to the retained previous
    /// generation.
    RolledBack,
    /// All generations were removed and the marker was tombstoned.
    Retired,
    /// The requested action was already satisfied (e.g. `retire` on an
    /// already-retired kind); no mutation occurred.
    VerifiedClean,
    /// The request was refused before any mutation (e.g. `rotate`
    /// requested for a never-provisioned kind).
    Denied,
    /// The step aborted partway and failed closed. Never a silent
    /// partial mutation.
    FailedClosed,
}

/// Terminal disposition of the identity-bound tamper-guard marker for
/// this action, mirroring the `swtpm_dir.rs` marker-result shape but
/// kept as an independent type (this module never imports from
/// `swtpm_dir.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerResult {
    /// A fresh marker recording the trusted generation identity was
    /// written.
    Created,
    /// An existing marker verified against the live directory's
    /// current identity.
    Verified,
    /// The marker was rewritten to record a completed retirement.
    Tombstoned,
    /// The action never reached the marker step (denied before any
    /// filesystem mutation).
    Unchanged,
    /// The marker was absent-after-prior-provision, tampered, or
    /// otherwise failed identity verification. The action fails closed.
    FailedClosed,
}

/// Always-present, path-free context threaded through every
/// constructor so a per-status variant only supplies the fields that
/// vary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsLifecycleAuditContext {
    pub vm_id: String,
    pub kind: SecretKind,
    pub action: LifecycleAction,
    /// FNV1a-64 hash of the per-kind secrets-state directory path
    /// (parity with [`crate::ops::hosts::stable_hash_str`]).
    pub base_dir_hash: String,
}

/// Path-free, material-free terminal audit record for one secrets
/// lifecycle attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretsLifecycleAuditFields {
    pub schema_version: u32,
    pub vm_id: String,
    pub kind: SecretKind,
    pub action: LifecycleAction,
    pub base_dir_hash: String,
    pub result: LifecycleResult,
    pub marker_result: MarkerResult,
    /// The active generation number after this action, when one
    /// exists (`None` after a successful `retire`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    /// On-disk generations retained for rollback after this action
    /// (excludes the active generation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_generations: Vec<u32>,
    /// SHA-256 hex digest of the material this action wrote, when the
    /// action wrote material (`provision`/`rotate`). `None` for
    /// `rollback`/`retire`/failed attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_digest_hex: Option<String>,
    /// Closed-set, path-free reason slug. Present exactly when
    /// `result` is `Denied` or `FailedClosed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_reason: Option<String>,
}

/// Errors [`SecretsLifecycleAuditFields::validate`] can report. Kept
/// separate from [`crate::ops::OpError`] because this module has no
/// dependency on the rest of `ops::` beyond `serde`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsLifecycleAuditError {
    SchemaVersionMismatch { found: u32 },
    InvalidVmId,
    MissingGeneration,
    UnexpectedGeneration,
    GenerationIsZero,
    RetainedGenerationsTooLarge { len: usize },
    RetainedGenerationsNotUnique,
    RetainedGenerationsIncludeActive,
    MissingFailReason,
    UnexpectedFailReason,
    MarkerResultInconsistentWithResult,
    InvalidMaterialDigest,
    UnexpectedMaterialDigest,
}

impl std::fmt::Display for SecretsLifecycleAuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaVersionMismatch { found } => {
                write!(
                    f,
                    "schema_version {found} != {SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION}"
                )
            }
            Self::InvalidVmId => write!(f, "vm_id is empty or contains a path separator"),
            Self::MissingGeneration => write!(f, "result requires a generation number"),
            Self::UnexpectedGeneration => write!(f, "result must not carry a generation number"),
            Self::GenerationIsZero => write!(f, "generation numbers start at 1"),
            Self::RetainedGenerationsTooLarge { len } => write!(
                f,
                "retained_generations length {len} exceeds {MAX_AUDITED_RETAINED_GENERATIONS}"
            ),
            Self::RetainedGenerationsNotUnique => write!(f, "retained_generations has duplicates"),
            Self::RetainedGenerationsIncludeActive => {
                write!(f, "retained_generations must exclude the active generation")
            }
            Self::MissingFailReason => write!(f, "fail_reason required for this result"),
            Self::UnexpectedFailReason => write!(f, "fail_reason forbidden for this result"),
            Self::MarkerResultInconsistentWithResult => {
                write!(f, "marker_result is inconsistent with result")
            }
            Self::InvalidMaterialDigest => {
                write!(f, "material_digest_hex must be 64 lowercase hex characters")
            }
            Self::UnexpectedMaterialDigest => {
                write!(f, "material_digest_hex forbidden for this action/result")
            }
        }
    }
}

impl std::error::Error for SecretsLifecycleAuditError {}

fn valid_vm_id(vm_id: &str) -> bool {
    !vm_id.is_empty() && !vm_id.contains('/') && !vm_id.contains('\0')
}

fn valid_material_digest_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl SecretsLifecycleAuditFields {
    /// Validate every cross-field invariant the JSON drift / schema
    /// tests rely on to reject a hand-built invalid record. Every
    /// constructor below returns a record that already passes this.
    pub fn validate(&self) -> Result<(), SecretsLifecycleAuditError> {
        if self.schema_version != SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION {
            return Err(SecretsLifecycleAuditError::SchemaVersionMismatch {
                found: self.schema_version,
            });
        }
        if !valid_vm_id(&self.vm_id) {
            return Err(SecretsLifecycleAuditError::InvalidVmId);
        }
        if let Some(generation) = self.generation
            && generation == 0
        {
            return Err(SecretsLifecycleAuditError::GenerationIsZero);
        }
        if self.retained_generations.len() > MAX_AUDITED_RETAINED_GENERATIONS {
            return Err(SecretsLifecycleAuditError::RetainedGenerationsTooLarge {
                len: self.retained_generations.len(),
            });
        }
        {
            let mut seen = std::collections::HashSet::new();
            for generation in &self.retained_generations {
                if !seen.insert(*generation) {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsNotUnique);
                }
            }
        }
        if let Some(generation) = self.generation
            && self.retained_generations.contains(&generation)
        {
            return Err(SecretsLifecycleAuditError::RetainedGenerationsIncludeActive);
        }

        match self.result {
            LifecycleResult::Created | LifecycleResult::Rotated | LifecycleResult::RolledBack => {
                if self.generation.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingGeneration);
                }
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if matches!(
                    self.marker_result,
                    MarkerResult::FailedClosed | MarkerResult::Unchanged | MarkerResult::Tombstoned
                ) {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
            }
            LifecycleResult::Retired => {
                if self.generation.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedGeneration);
                }
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if self.marker_result != MarkerResult::Tombstoned {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
                if self.material_digest_hex.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                }
            }
            LifecycleResult::VerifiedClean => {
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if self.material_digest_hex.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                }
            }
            LifecycleResult::Denied => {
                if self.fail_reason.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingFailReason);
                }
                if self.marker_result != MarkerResult::Unchanged {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
                if self.material_digest_hex.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                }
            }
            LifecycleResult::FailedClosed => {
                if self.fail_reason.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingFailReason);
                }
            }
        }

        // Only provision/rotate ever write fresh material.
        if let Some(digest) = &self.material_digest_hex {
            if !valid_material_digest_hex(digest) {
                return Err(SecretsLifecycleAuditError::InvalidMaterialDigest);
            }
            if !matches!(
                self.action,
                LifecycleAction::Provision | LifecycleAction::Rotate
            ) {
                return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
            }
        }

        Ok(())
    }

    fn base(ctx: &SecretsLifecycleAuditContext) -> Self {
        Self {
            schema_version: SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION,
            vm_id: ctx.vm_id.clone(),
            kind: ctx.kind,
            action: ctx.action,
            base_dir_hash: ctx.base_dir_hash.clone(),
            result: LifecycleResult::VerifiedClean,
            marker_result: MarkerResult::Unchanged,
            generation: None,
            retained_generations: Vec::new(),
            material_digest_hex: None,
            fail_reason: None,
        }
    }

    /// A fresh generation 1 was provisioned.
    pub fn provisioned(ctx: &SecretsLifecycleAuditContext, material_digest_hex: String) -> Self {
        Self {
            result: LifecycleResult::Created,
            marker_result: MarkerResult::Created,
            generation: Some(1),
            material_digest_hex: Some(material_digest_hex),
            ..Self::base(ctx)
        }
    }

    /// A new generation was created and activated.
    pub fn rotated(
        ctx: &SecretsLifecycleAuditContext,
        generation: u32,
        retained_generations: Vec<u32>,
        material_digest_hex: String,
    ) -> Self {
        Self {
            result: LifecycleResult::Rotated,
            marker_result: MarkerResult::Verified,
            generation: Some(generation),
            retained_generations,
            material_digest_hex: Some(material_digest_hex),
            ..Self::base(ctx)
        }
    }

    /// `current` was swapped back to a retained prior generation.
    pub fn rolled_back(
        ctx: &SecretsLifecycleAuditContext,
        generation: u32,
        retained_generations: Vec<u32>,
    ) -> Self {
        Self {
            result: LifecycleResult::RolledBack,
            marker_result: MarkerResult::Verified,
            generation: Some(generation),
            retained_generations,
            ..Self::base(ctx)
        }
    }

    /// Every generation was removed and the marker tombstoned.
    pub fn retired(ctx: &SecretsLifecycleAuditContext) -> Self {
        Self {
            result: LifecycleResult::Retired,
            marker_result: MarkerResult::Tombstoned,
            ..Self::base(ctx)
        }
    }

    /// The action was already satisfied; no mutation occurred.
    pub fn verified_clean(ctx: &SecretsLifecycleAuditContext, generation: Option<u32>) -> Self {
        Self {
            result: LifecycleResult::VerifiedClean,
            marker_result: MarkerResult::Verified,
            generation,
            ..Self::base(ctx)
        }
    }

    /// The action was refused before any filesystem mutation.
    pub fn denied(ctx: &SecretsLifecycleAuditContext, reason: &'static str) -> Self {
        Self {
            result: LifecycleResult::Denied,
            marker_result: MarkerResult::Unchanged,
            fail_reason: Some(reason.to_owned()),
            ..Self::base(ctx)
        }
    }

    /// The action aborted partway and failed closed.
    pub fn failed(
        ctx: &SecretsLifecycleAuditContext,
        marker_result: MarkerResult,
        reason: &'static str,
    ) -> Self {
        Self {
            result: LifecycleResult::FailedClosed,
            marker_result,
            fail_reason: Some(reason.to_owned()),
            ..Self::base(ctx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(action: LifecycleAction) -> SecretsLifecycleAuditContext {
        SecretsLifecycleAuditContext {
            vm_id: "work".to_owned(),
            kind: SecretKind::GuestSigningKey,
            action,
            base_dir_hash: "0123456789abcdef".to_owned(),
        }
    }

    fn digest() -> String {
        "a".repeat(64)
    }

    #[test]
    fn secret_kind_slugs_are_stable_and_distinct() {
        let slugs = [
            SecretKind::TpmBoundCredential.as_slug(),
            SecretKind::GuestSigningKey.as_slug(),
            SecretKind::SecurityKeyChannelState.as_slug(),
        ];
        assert_eq!(
            slugs,
            [
                "tpm-bound-credential",
                "guest-signing-key",
                "security-key-channel-state"
            ]
        );
        let unique: std::collections::HashSet<_> = slugs.iter().collect();
        assert_eq!(unique.len(), slugs.len());
    }

    #[test]
    fn provisioned_validates() {
        let record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), digest());
        record.validate().expect("provisioned record must validate");
        assert_eq!(record.generation, Some(1));
        assert!(record.retained_generations.is_empty());
    }

    #[test]
    fn rotated_validates_and_excludes_active_from_retained() {
        let record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            vec![1],
            digest(),
        );
        record.validate().expect("rotated record must validate");

        let mut broken = record.clone();
        broken.retained_generations.push(2);
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsIncludeActive)
        );
    }

    #[test]
    fn rolled_back_validates() {
        let record =
            SecretsLifecycleAuditFields::rolled_back(&ctx(LifecycleAction::Rollback), 1, vec![2]);
        record.validate().expect("rolled back record must validate");
    }

    #[test]
    fn retired_validates_and_forbids_generation() {
        let record = SecretsLifecycleAuditFields::retired(&ctx(LifecycleAction::Retire));
        record.validate().expect("retired record must validate");

        let mut broken = record.clone();
        broken.generation = Some(1);
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedGeneration)
        );
    }

    #[test]
    fn denied_requires_fail_reason_and_unchanged_marker() {
        let record = SecretsLifecycleAuditFields::denied(
            &ctx(LifecycleAction::Rotate),
            "secrets-lifecycle-not-provisioned",
        );
        record.validate().expect("denied record must validate");

        let mut broken = record.clone();
        broken.fail_reason = None;
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::MissingFailReason)
        );

        let mut broken_marker = record.clone();
        broken_marker.marker_result = MarkerResult::FailedClosed;
        assert_eq!(
            broken_marker.validate(),
            Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult)
        );
    }

    #[test]
    fn failed_closed_requires_fail_reason() {
        let record = SecretsLifecycleAuditFields::failed(
            &ctx(LifecycleAction::Rotate),
            MarkerResult::FailedClosed,
            "secrets-lifecycle-marker-tampered",
        );
        record.validate().expect("failed record must validate");

        let mut broken = record.clone();
        broken.fail_reason = None;
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::MissingFailReason)
        );
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), digest());
        record.schema_version = 2;
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::SchemaVersionMismatch { found: 2 })
        );
    }

    #[test]
    fn invalid_vm_id_is_rejected() {
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), digest());
        for bad in ["", "work/vm", "wor\0k"] {
            record.vm_id = bad.to_owned();
            assert_eq!(
                record.validate(),
                Err(SecretsLifecycleAuditError::InvalidVmId)
            );
        }
    }

    #[test]
    fn generation_zero_is_rejected() {
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), digest());
        record.generation = Some(0);
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::GenerationIsZero)
        );
    }

    #[test]
    fn retained_generations_bounds_and_uniqueness() {
        let mut record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            100,
            (1..=MAX_AUDITED_RETAINED_GENERATIONS as u32 + 1).collect(),
            digest(),
        );
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsTooLarge {
                len: MAX_AUDITED_RETAINED_GENERATIONS + 1
            })
        );

        record.retained_generations = vec![1, 1];
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsNotUnique)
        );
    }

    #[test]
    fn material_digest_is_scoped_to_provision_and_rotate_actions() {
        let mut record =
            SecretsLifecycleAuditFields::rolled_back(&ctx(LifecycleAction::Rollback), 1, vec![2]);
        record.material_digest_hex = Some(digest());
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest)
        );
    }

    #[test]
    fn material_digest_must_be_lowercase_hex_64() {
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), digest());
        for bad in ["", "AA", &"a".repeat(63), &"g".repeat(64), &"A".repeat(64)] {
            record.material_digest_hex = Some(bad.to_owned());
            assert_eq!(
                record.validate(),
                Err(SecretsLifecycleAuditError::InvalidMaterialDigest)
            );
        }
    }

    #[test]
    fn serde_round_trip_preserves_every_field() {
        let record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            3,
            vec![2],
            digest(),
        );
        let json = serde_json::to_string(&record).expect("serialize");
        let decoded: SecretsLifecycleAuditFields =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, record);
    }

    #[test]
    fn verified_clean_allows_absent_or_present_generation() {
        let without =
            SecretsLifecycleAuditFields::verified_clean(&ctx(LifecycleAction::Retire), None);
        without
            .validate()
            .expect("verified clean without generation must validate");
        let with =
            SecretsLifecycleAuditFields::verified_clean(&ctx(LifecycleAction::Rotate), Some(4));
        with.validate()
            .expect("verified clean with generation must validate");
    }
}
