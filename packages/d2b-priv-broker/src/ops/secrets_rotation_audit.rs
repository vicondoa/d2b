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
//! and closed-set result/marker/reason enums. **`fail_reason` is a
//! closed enum, not a free-form string** — this is a deliberate
//! hardening over an earlier draft of this module that carried
//! `Option<String>` populated from convention-only `&'static str`
//! constants; nothing outside this file's [`FailReason`] enum can ever
//! reach the audit surface.
//!
//! # Status
//!
//! This schema, and the [`crate::ops::secrets_lifecycle`] engine that
//! produces it, are **not wired into any live broker dispatch path or
//! `crate::audit::AuditLog` sink**. See that module's "Integration
//! wiring points" doc section for the exact list of follow-up steps a
//! future integrator must perform before any of this is observable in
//! a running broker's audit log.

use serde::{Deserialize, Serialize};

/// Schema version for the secrets-lifecycle terminal audit record.
///
/// Bumped from `1` to `2` for the transaction/recovery + strengthened
/// identity-binding redesign: `generation` (u32) became `lineage_epoch`
/// (u64) with a companion `high_water_epoch`, `retained_generations`
/// became `u64`-keyed, `fail_reason` became a closed enum instead of a
/// free-form string, and `recovered_prior_transaction` was added.
pub const SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION: u32 = 2;

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

/// Closed set of path-free, redaction-safe failure/refusal reasons.
/// Every [`SecretsLifecycleAuditFields::denied`] /
/// [`SecretsLifecycleAuditFields::failed`] call site in
/// `secrets_lifecycle.rs` constructs one of these variants directly —
/// there is no code path that can place an arbitrary string or a raw
/// filesystem path onto the audit surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    InvalidVmId,
    PathDerivationFailed,
    SecretsDirOpenFailed,
    MarkerTreeOpenFailed,
    MarkerWriteFailed,
    MarkerTamperedOrMissingMaterial,
    AlreadyProvisioned,
    AlreadyRetired,
    NotProvisioned,
    NoRollbackTarget,
    PreviouslyProvisionedMaterialMissing,
    GenerationConflict,
    MaterialWriteFailed,
    CurrentSwapFailed,
    InvalidMaterial,
    /// The cross-process per-`(vm, kind)` exclusive lock could not be
    /// acquired within the bounded wait budget.
    LockUnavailable,
    /// The durable transaction/recovery log (`txlog`) could not be
    /// written or fsynced.
    IntentWriteFailed,
    /// A leftover `txlog` exists but is not well-formed JSON, fails
    /// schema validation, or names a `(vm, kind)` other than the one
    /// it was found under.
    IntentCorrupt,
    /// Resuming a leftover in-flight transaction found on-disk content
    /// that does not match what the transaction log recorded before
    /// the crash (e.g. a digest mismatch on the staged/committed
    /// epoch). Recovery refuses to guess and fails closed without
    /// touching `current` or the marker.
    RecoveryContentMismatch,
    /// Resuming a leftover in-flight transaction found the filesystem
    /// in a state recovery cannot map to any phase of the recorded
    /// transaction (e.g. `current` points somewhere the log did not
    /// expect). Recovery fails closed rather than guessing.
    RecoveryAmbiguous,
    /// A collision-resistant staging name could not be allocated
    /// within the bounded retry budget (astronomically unlikely; ever
    /// observing this indicates a broken randomness source).
    StagingNameExhausted,
    /// Retirement's full anchored-tree enumeration found an entry it
    /// could not account for (unexpected name, unexpected type, or a
    /// material file with more than one hard link) and refused to
    /// delete anything.
    RetirementTreeAnomaly,
    /// Retirement removed every entry it validated but the generations
    /// tree was not observably empty afterward.
    RetirementNotProvablyEmpty,
    /// The live active-generation directory's owner uid/gid does not
    /// match the marker's recorded identity.
    IdentityOwnerMismatch,
    /// The live active-generation directory's mode does not match the
    /// marker's recorded identity.
    IdentityModeMismatch,
    /// The live active-generation directory carries (or lost) an
    /// extended POSIX ACL relative to what the marker recorded.
    IdentityAclMismatch,
    /// The live active-generation directory's material file link
    /// count is not exactly 1 (a hard-link plant), or otherwise does
    /// not match the marker's recorded identity.
    IdentityLinkCountMismatch,
    /// The live active-generation material's SHA-256 digest does not
    /// match the marker's recorded identity.
    IdentityDigestMismatch,
    /// The live active-generation material's SHA-256 digest matches
    /// the marker's recorded identity, but the `(dev, ino)` pair does
    /// not — a hard-link or directory-swap tamper that byte-content
    /// comparison alone cannot see (a replacement file with identical
    /// bytes but a different physical inode).
    IdentityInodeMismatch,
    /// `current` does not literally resolve (by name, not just by
    /// coincidental dev/ino) to the epoch the marker records as active.
    IdentityCurrentTargetMismatch,
    /// A newly computed high-water epoch would not be strictly greater
    /// than the marker's previously committed high-water epoch — a
    /// monotonicity invariant violation this module refuses to persist.
    HighWaterRegressed,
    /// An `fsync`/parent-directory-sync of a file or directory this
    /// module just durably wrote returned an error. Every phase
    /// advancement in the transaction/recovery state machine is
    /// blocked on this succeeding — a silently-ignored fsync failure
    /// would let a "durable" phase transition be lost on crash.
    FsyncFailed,
    /// The lock file, its containing directory, or another broker-
    /// trusted metadata path (never a `material` leaf) was found with
    /// an owner, group, mode, type, or link count that does not match
    /// this process's own (trusted broker) identity — i.e. it was not
    /// created/managed by this code path and must not be trusted for
    /// mutual exclusion or transaction bookkeeping.
    BrokerOwnershipViolation,
}

impl FailReason {
    /// Stable slug matching the enum variant's `snake_case` wire form,
    /// safe for any Debug/log/audit surface (never a path or secret).
    pub fn as_slug(self) -> &'static str {
        match self {
            Self::InvalidVmId => "invalid_vm_id",
            Self::PathDerivationFailed => "path_derivation_failed",
            Self::SecretsDirOpenFailed => "secrets_dir_open_failed",
            Self::MarkerTreeOpenFailed => "marker_tree_open_failed",
            Self::MarkerWriteFailed => "marker_write_failed",
            Self::MarkerTamperedOrMissingMaterial => "marker_tampered_or_missing_material",
            Self::AlreadyProvisioned => "already_provisioned",
            Self::AlreadyRetired => "already_retired",
            Self::NotProvisioned => "not_provisioned",
            Self::NoRollbackTarget => "no_rollback_target",
            Self::PreviouslyProvisionedMaterialMissing => "previously_provisioned_material_missing",
            Self::GenerationConflict => "generation_conflict",
            Self::MaterialWriteFailed => "material_write_failed",
            Self::CurrentSwapFailed => "current_swap_failed",
            Self::InvalidMaterial => "invalid_material",
            Self::LockUnavailable => "lock_unavailable",
            Self::IntentWriteFailed => "intent_write_failed",
            Self::IntentCorrupt => "intent_corrupt",
            Self::RecoveryContentMismatch => "recovery_content_mismatch",
            Self::RecoveryAmbiguous => "recovery_ambiguous",
            Self::StagingNameExhausted => "staging_name_exhausted",
            Self::RetirementTreeAnomaly => "retirement_tree_anomaly",
            Self::RetirementNotProvablyEmpty => "retirement_not_provably_empty",
            Self::IdentityOwnerMismatch => "identity_owner_mismatch",
            Self::IdentityModeMismatch => "identity_mode_mismatch",
            Self::IdentityAclMismatch => "identity_acl_mismatch",
            Self::IdentityLinkCountMismatch => "identity_link_count_mismatch",
            Self::IdentityDigestMismatch => "identity_digest_mismatch",
            Self::IdentityInodeMismatch => "identity_inode_mismatch",
            Self::IdentityCurrentTargetMismatch => "identity_current_target_mismatch",
            Self::HighWaterRegressed => "high_water_regressed",
            Self::FsyncFailed => "fsync_failed",
            Self::BrokerOwnershipViolation => "broker_ownership_violation",
        }
    }
}

impl std::fmt::Display for FailReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_slug())
    }
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
    /// The active lineage epoch after this action, when one exists
    /// (`None` after a successful `retire`, and for `Denied`/
    /// `FailedClosed` results). This is the monotonic identity anchor
    /// (see `secrets_lifecycle::MarkerData::high_water_epoch`) — never
    /// simply "current epoch number + 1", so a rotate issued after a
    /// rollback can never collide with (or silently resurrect) a
    /// still-materialized older epoch directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_epoch: Option<u64>,
    /// The monotonic high-water epoch recorded by the marker after
    /// this action (present exactly when `lineage_epoch` is present,
    /// and additionally present after a successful `retire` so an
    /// auditor can confirm a subsequent re-provision's first epoch is
    /// still strictly greater than every epoch this `(vm, kind)` ever
    /// used). Never decreases across the lifetime of a `(vm, kind)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_water_epoch: Option<u64>,
    /// On-disk generations retained for rollback after this action
    /// (excludes the active generation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_generations: Vec<u64>,
    /// SHA-256 hex digest of the material this action wrote, when the
    /// action wrote material (`provision`/`rotate`). `None` for
    /// `rollback`/`retire`/failed attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_digest_hex: Option<String>,
    /// Closed-set, path-free failure/refusal reason. Present exactly
    /// when `result` is `Denied` or `FailedClosed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_reason: Option<FailReason>,
    /// `true` when this action first had to resolve a leftover,
    /// crash-interrupted transaction (see
    /// `secrets_lifecycle::recover_in_flight_transaction`) before
    /// proceeding with the requested action. Always `false` on the
    /// common, no-crash path; surfaced so an operator can distinguish
    /// "this rotate ran cleanly" from "this rotate first had to finish
    /// or unwind a prior crashed rotate".
    #[serde(default)]
    pub recovered_prior_transaction: bool,
}

/// Errors [`SecretsLifecycleAuditFields::validate`] can report. Kept
/// separate from [`crate::ops::OpError`] because this module has no
/// dependency on the rest of `ops::` beyond `serde`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsLifecycleAuditError {
    SchemaVersionMismatch { found: u32 },
    InvalidVmId,
    MissingLineageEpoch,
    UnexpectedLineageEpoch,
    LineageEpochIsZero,
    MissingHighWaterEpoch,
    UnexpectedHighWaterEpoch,
    HighWaterBelowLineageEpoch,
    RetainedGenerationsTooLarge { len: usize },
    RetainedGenerationsNotUnique,
    RetainedGenerationsIncludeActive,
    RetainedGenerationsContainZero,
    RetainedGenerationsMustBeNonEmpty,
    RetainedGenerationsMustBeEmpty,
    MissingFailReason,
    UnexpectedFailReason,
    ActionResultIncompatible,
    MarkerResultInconsistentWithResult,
    InvalidMaterialDigest,
    UnexpectedMaterialDigest,
    MissingMaterialDigest,
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
            Self::MissingLineageEpoch => write!(f, "result requires a lineage_epoch"),
            Self::UnexpectedLineageEpoch => write!(f, "result must not carry a lineage_epoch"),
            Self::LineageEpochIsZero => write!(f, "lineage epochs start at 1"),
            Self::MissingHighWaterEpoch => write!(f, "result requires a high_water_epoch"),
            Self::UnexpectedHighWaterEpoch => write!(f, "result must not carry a high_water_epoch"),
            Self::HighWaterBelowLineageEpoch => {
                write!(f, "high_water_epoch must be >= lineage_epoch")
            }
            Self::RetainedGenerationsTooLarge { len } => write!(
                f,
                "retained_generations length {len} exceeds {MAX_AUDITED_RETAINED_GENERATIONS}"
            ),
            Self::RetainedGenerationsNotUnique => write!(f, "retained_generations has duplicates"),
            Self::RetainedGenerationsIncludeActive => {
                write!(f, "retained_generations must exclude the active generation")
            }
            Self::RetainedGenerationsContainZero => {
                write!(f, "retained_generations must not contain epoch 0")
            }
            Self::RetainedGenerationsMustBeNonEmpty => write!(
                f,
                "this action/result requires at least one retained generation"
            ),
            Self::RetainedGenerationsMustBeEmpty => {
                write!(f, "this action/result must not carry a retained generation")
            }
            Self::MissingFailReason => write!(f, "fail_reason required for this result"),
            Self::UnexpectedFailReason => write!(f, "fail_reason forbidden for this result"),
            Self::ActionResultIncompatible => {
                write!(f, "result is not a reachable outcome of this action")
            }
            Self::MarkerResultInconsistentWithResult => {
                write!(f, "marker_result is inconsistent with result")
            }
            Self::InvalidMaterialDigest => {
                write!(f, "material_digest_hex must be 64 lowercase hex characters")
            }
            Self::UnexpectedMaterialDigest => {
                write!(f, "material_digest_hex forbidden for this action/result")
            }
            Self::MissingMaterialDigest => {
                write!(f, "material_digest_hex required for this action/result")
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
    ///
    /// This enforces a **complete** `action` x `result` x
    /// `marker_result` compatibility matrix (not just per-field
    /// presence checks): every `(action, result)` pair not explicitly
    /// reachable from `secrets_lifecycle.rs` is rejected here too, so
    /// a hand-built or future-buggy record can never claim an
    /// impossible combination (e.g. `Provision` producing `RolledBack`).
    pub fn validate(&self) -> Result<(), SecretsLifecycleAuditError> {
        if self.schema_version != SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION {
            return Err(SecretsLifecycleAuditError::SchemaVersionMismatch {
                found: self.schema_version,
            });
        }
        if !valid_vm_id(&self.vm_id) {
            return Err(SecretsLifecycleAuditError::InvalidVmId);
        }
        if let Some(epoch) = self.lineage_epoch
            && epoch == 0
        {
            return Err(SecretsLifecycleAuditError::LineageEpochIsZero);
        }
        if self.retained_generations.len() > MAX_AUDITED_RETAINED_GENERATIONS {
            return Err(SecretsLifecycleAuditError::RetainedGenerationsTooLarge {
                len: self.retained_generations.len(),
            });
        }
        {
            let mut seen = std::collections::HashSet::new();
            for epoch in &self.retained_generations {
                if !seen.insert(*epoch) {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsNotUnique);
                }
            }
        }
        if self.retained_generations.contains(&0) {
            return Err(SecretsLifecycleAuditError::RetainedGenerationsContainZero);
        }
        if let Some(epoch) = self.lineage_epoch
            && self.retained_generations.contains(&epoch)
        {
            return Err(SecretsLifecycleAuditError::RetainedGenerationsIncludeActive);
        }
        if let (Some(epoch), Some(high_water)) = (self.lineage_epoch, self.high_water_epoch)
            && high_water < epoch
        {
            return Err(SecretsLifecycleAuditError::HighWaterBelowLineageEpoch);
        }

        // Action x result reachability matrix: only these pairs are
        // ever constructed by `secrets_lifecycle.rs`. Anything else
        // (e.g. `Provision` -> `RolledBack`) is rejected outright,
        // before the per-result field checks below even run.
        let action_result_ok = matches!(
            (self.action, self.result),
            (LifecycleAction::Provision, LifecycleResult::Created)
                | (LifecycleAction::Provision, LifecycleResult::Denied)
                | (LifecycleAction::Provision, LifecycleResult::FailedClosed)
                | (LifecycleAction::Rotate, LifecycleResult::Rotated)
                | (LifecycleAction::Rotate, LifecycleResult::Denied)
                | (LifecycleAction::Rotate, LifecycleResult::FailedClosed)
                | (LifecycleAction::Rollback, LifecycleResult::RolledBack)
                | (LifecycleAction::Rollback, LifecycleResult::Denied)
                | (LifecycleAction::Rollback, LifecycleResult::FailedClosed)
                | (LifecycleAction::Retire, LifecycleResult::Retired)
                | (LifecycleAction::Retire, LifecycleResult::VerifiedClean)
                | (LifecycleAction::Retire, LifecycleResult::Denied)
                | (LifecycleAction::Retire, LifecycleResult::FailedClosed)
        );
        if !action_result_ok {
            return Err(SecretsLifecycleAuditError::ActionResultIncompatible);
        }

        match self.result {
            LifecycleResult::Created => {
                if self.lineage_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingLineageEpoch);
                }
                if self.high_water_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingHighWaterEpoch);
                }
                if !self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty);
                }
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if self.marker_result != MarkerResult::Created {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
                if self.material_digest_hex.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingMaterialDigest);
                }
            }
            LifecycleResult::Rotated | LifecycleResult::RolledBack => {
                if self.lineage_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingLineageEpoch);
                }
                if self.high_water_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingHighWaterEpoch);
                }
                if self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeNonEmpty);
                }
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if self.marker_result != MarkerResult::Verified {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
                let digest_required = self.result == LifecycleResult::Rotated;
                match (&self.material_digest_hex, digest_required) {
                    (None, true) => {
                        return Err(SecretsLifecycleAuditError::MissingMaterialDigest);
                    }
                    (Some(_), false) => {
                        return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                    }
                    _ => {}
                }
            }
            LifecycleResult::Retired => {
                if self.lineage_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch);
                }
                if self.high_water_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingHighWaterEpoch);
                }
                if !self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty);
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
                // Only reachable for `Retire` (already-retired /
                // never-provisioned) per the action/result matrix
                // above.
                if self.lineage_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch);
                }
                if self.high_water_epoch.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingHighWaterEpoch);
                }
                if !self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty);
                }
                if self.fail_reason.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedFailReason);
                }
                if self.marker_result != MarkerResult::Verified {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
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
                if self.lineage_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch);
                }
                if self.high_water_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedHighWaterEpoch);
                }
                if !self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty);
                }
                if self.material_digest_hex.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                }
            }
            LifecycleResult::FailedClosed => {
                if self.fail_reason.is_none() {
                    return Err(SecretsLifecycleAuditError::MissingFailReason);
                }
                // A failed-closed action never durably activated
                // anything (finding: "never return an error after
                // silently activating unrecoverable state"), so it
                // must never carry epoch/retained-generation state —
                // and its marker disposition is always exactly
                // `FailedClosed`, never `Verified`/`Created`/
                // `Tombstoned`/`Unchanged`.
                if self.marker_result != MarkerResult::FailedClosed {
                    return Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult);
                }
                if self.lineage_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch);
                }
                if self.high_water_epoch.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedHighWaterEpoch);
                }
                if !self.retained_generations.is_empty() {
                    return Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty);
                }
                if self.material_digest_hex.is_some() {
                    return Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest);
                }
            }
        }

        if let Some(digest) = &self.material_digest_hex
            && !valid_material_digest_hex(digest)
        {
            return Err(SecretsLifecycleAuditError::InvalidMaterialDigest);
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
            lineage_epoch: None,
            high_water_epoch: None,
            retained_generations: Vec::new(),
            material_digest_hex: None,
            fail_reason: None,
            recovered_prior_transaction: false,
        }
    }

    /// A fresh generation 1 was provisioned.
    pub fn provisioned(
        ctx: &SecretsLifecycleAuditContext,
        high_water_epoch: u64,
        material_digest_hex: String,
        recovered_prior_transaction: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::Created,
            marker_result: MarkerResult::Created,
            lineage_epoch: Some(1),
            high_water_epoch: Some(high_water_epoch),
            material_digest_hex: Some(material_digest_hex),
            recovered_prior_transaction,
            ..Self::base(ctx)
        }
    }

    /// A new generation was created and activated.
    #[allow(clippy::too_many_arguments)]
    pub fn rotated(
        ctx: &SecretsLifecycleAuditContext,
        lineage_epoch: u64,
        high_water_epoch: u64,
        retained_generations: Vec<u64>,
        material_digest_hex: String,
        recovered_prior_transaction: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::Rotated,
            marker_result: MarkerResult::Verified,
            lineage_epoch: Some(lineage_epoch),
            high_water_epoch: Some(high_water_epoch),
            retained_generations,
            material_digest_hex: Some(material_digest_hex),
            recovered_prior_transaction,
            ..Self::base(ctx)
        }
    }

    /// `current` was swapped back to a retained prior generation.
    pub fn rolled_back(
        ctx: &SecretsLifecycleAuditContext,
        lineage_epoch: u64,
        high_water_epoch: u64,
        retained_generations: Vec<u64>,
        recovered_prior_transaction: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::RolledBack,
            marker_result: MarkerResult::Verified,
            lineage_epoch: Some(lineage_epoch),
            high_water_epoch: Some(high_water_epoch),
            retained_generations,
            recovered_prior_transaction,
            ..Self::base(ctx)
        }
    }

    /// Every generation was removed and the marker tombstoned.
    pub fn retired(
        ctx: &SecretsLifecycleAuditContext,
        high_water_epoch: u64,
        recovered_prior_transaction: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::Retired,
            marker_result: MarkerResult::Tombstoned,
            high_water_epoch: Some(high_water_epoch),
            recovered_prior_transaction,
            ..Self::base(ctx)
        }
    }

    /// The action was already satisfied; no mutation occurred. Only
    /// reachable for `retire` (never-provisioned or already-retired).
    pub fn verified_clean(ctx: &SecretsLifecycleAuditContext, high_water_epoch: u64) -> Self {
        Self {
            result: LifecycleResult::VerifiedClean,
            marker_result: MarkerResult::Verified,
            high_water_epoch: Some(high_water_epoch),
            ..Self::base(ctx)
        }
    }

    /// The action was refused before any filesystem mutation.
    pub fn denied(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> Self {
        Self {
            result: LifecycleResult::Denied,
            marker_result: MarkerResult::Unchanged,
            fail_reason: Some(reason),
            ..Self::base(ctx)
        }
    }

    /// The action aborted partway and failed closed.
    pub fn failed(
        ctx: &SecretsLifecycleAuditContext,
        marker_result: MarkerResult,
        reason: FailReason,
    ) -> Self {
        Self {
            result: LifecycleResult::FailedClosed,
            marker_result,
            fail_reason: Some(reason),
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
    fn fail_reason_slugs_are_stable_and_distinct() {
        let variants = [
            FailReason::InvalidVmId,
            FailReason::PathDerivationFailed,
            FailReason::SecretsDirOpenFailed,
            FailReason::MarkerTreeOpenFailed,
            FailReason::MarkerWriteFailed,
            FailReason::MarkerTamperedOrMissingMaterial,
            FailReason::AlreadyProvisioned,
            FailReason::AlreadyRetired,
            FailReason::NotProvisioned,
            FailReason::NoRollbackTarget,
            FailReason::PreviouslyProvisionedMaterialMissing,
            FailReason::GenerationConflict,
            FailReason::MaterialWriteFailed,
            FailReason::CurrentSwapFailed,
            FailReason::InvalidMaterial,
            FailReason::LockUnavailable,
            FailReason::IntentWriteFailed,
            FailReason::IntentCorrupt,
            FailReason::RecoveryContentMismatch,
            FailReason::RecoveryAmbiguous,
            FailReason::StagingNameExhausted,
            FailReason::RetirementTreeAnomaly,
            FailReason::RetirementNotProvablyEmpty,
            FailReason::IdentityOwnerMismatch,
            FailReason::IdentityModeMismatch,
            FailReason::IdentityAclMismatch,
            FailReason::IdentityLinkCountMismatch,
            FailReason::IdentityDigestMismatch,
            FailReason::IdentityInodeMismatch,
            FailReason::IdentityCurrentTargetMismatch,
            FailReason::HighWaterRegressed,
            FailReason::FsyncFailed,
            FailReason::BrokerOwnershipViolation,
        ];
        let slugs: Vec<&str> = variants.iter().map(|v| v.as_slug()).collect();
        let unique: std::collections::HashSet<_> = slugs.iter().collect();
        assert_eq!(
            unique.len(),
            slugs.len(),
            "every FailReason variant must have a distinct slug"
        );
        for slug in &slugs {
            assert!(
                slug.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "slug {slug:?} must be snake_case ascii only (path-free, redaction-safe)"
            );
        }
    }

    #[test]
    fn provisioned_validates() {
        let record = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        record.validate().expect("provisioned record must validate");
        assert_eq!(record.lineage_epoch, Some(1));
        assert_eq!(record.high_water_epoch, Some(1));
        assert!(record.retained_generations.is_empty());
    }

    #[test]
    fn rotated_validates_and_excludes_active_from_retained() {
        let record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            2,
            vec![1],
            digest(),
            false,
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
    fn rotated_requires_nonempty_retained_generations() {
        let mut record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            2,
            vec![1],
            digest(),
            false,
        );
        record.retained_generations.clear();
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeNonEmpty)
        );
    }

    #[test]
    fn rolled_back_validates_and_requires_retained_generation() {
        let record = SecretsLifecycleAuditFields::rolled_back(
            &ctx(LifecycleAction::Rollback),
            1,
            2,
            vec![2],
            false,
        );
        record.validate().expect("rolled back record must validate");

        let mut broken = record.clone();
        broken.retained_generations.clear();
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeNonEmpty)
        );
    }

    #[test]
    fn rolled_back_forbids_material_digest() {
        let mut record = SecretsLifecycleAuditFields::rolled_back(
            &ctx(LifecycleAction::Rollback),
            1,
            2,
            vec![2],
            false,
        );
        record.material_digest_hex = Some(digest());
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest)
        );
    }

    #[test]
    fn retired_validates_and_forbids_lineage_epoch() {
        let record = SecretsLifecycleAuditFields::retired(&ctx(LifecycleAction::Retire), 5, false);
        record.validate().expect("retired record must validate");

        let mut broken = record.clone();
        broken.lineage_epoch = Some(1);
        assert_eq!(
            broken.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch)
        );
    }

    #[test]
    fn denied_requires_fail_reason_and_unchanged_marker() {
        let record = SecretsLifecycleAuditFields::denied(
            &ctx(LifecycleAction::Rotate),
            FailReason::NotProvisioned,
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
            FailReason::MarkerTamperedOrMissingMaterial,
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
    fn failed_closed_rejects_non_failed_closed_marker_result() {
        let mut record = SecretsLifecycleAuditFields::failed(
            &ctx(LifecycleAction::Retire),
            MarkerResult::FailedClosed,
            FailReason::RetirementTreeAnomaly,
        );
        record
            .validate()
            .expect("baseline failed record must validate");

        record.marker_result = MarkerResult::Verified;
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::MarkerResultInconsistentWithResult)
        );
    }

    #[test]
    fn failed_closed_rejects_leftover_epoch_or_retained_state() {
        let base = SecretsLifecycleAuditFields::failed(
            &ctx(LifecycleAction::Rotate),
            MarkerResult::FailedClosed,
            FailReason::HighWaterRegressed,
        );

        let mut with_lineage = base.clone();
        with_lineage.lineage_epoch = Some(1);
        assert_eq!(
            with_lineage.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedLineageEpoch)
        );

        let mut with_high_water = base.clone();
        with_high_water.high_water_epoch = Some(1);
        assert_eq!(
            with_high_water.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedHighWaterEpoch)
        );

        let mut with_retained = base;
        with_retained.retained_generations = vec![1];
        assert_eq!(
            with_retained.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsMustBeEmpty)
        );
    }

    /// `retire` + `Denied` is a genuinely reachable pair (an invalid
    /// `vm_id`, or a lock-acquisition failure, denies a retire attempt
    /// exactly like every other action) — the action/result matrix
    /// above must accept it, not just the three previously-listed
    /// retire outcomes.
    #[test]
    fn retire_denied_is_a_reachable_and_valid_pair() {
        let record = SecretsLifecycleAuditFields::denied(
            &ctx(LifecycleAction::Retire),
            FailReason::InvalidVmId,
        );
        record
            .validate()
            .expect("retire+Denied must be a valid, reachable action/result pair");
        assert_eq!(record.result, LifecycleResult::Denied);
        assert_eq!(record.action, LifecycleAction::Retire);
    }

    #[test]
    fn retained_generations_zero_is_rejected() {
        let mut record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            2,
            vec![0],
            digest(),
            false,
        );
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::RetainedGenerationsContainZero)
        );
        record.retained_generations = vec![1];
        record
            .validate()
            .expect("nonzero retained generation is valid");
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let mut record = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        record.schema_version = 1;
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::SchemaVersionMismatch { found: 1 })
        );
    }

    #[test]
    fn invalid_vm_id_is_rejected() {
        let mut record = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        for bad in ["", "work/vm", "wor\0k"] {
            record.vm_id = bad.to_owned();
            assert_eq!(
                record.validate(),
                Err(SecretsLifecycleAuditError::InvalidVmId)
            );
        }
    }

    #[test]
    fn lineage_epoch_zero_is_rejected() {
        let mut record = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        record.lineage_epoch = Some(0);
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::LineageEpochIsZero)
        );
    }

    #[test]
    fn high_water_below_lineage_epoch_is_rejected() {
        let mut record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            5,
            5,
            vec![4],
            digest(),
            false,
        );
        record.high_water_epoch = Some(4);
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::HighWaterBelowLineageEpoch)
        );
    }

    #[test]
    fn retained_generations_bounds_and_uniqueness() {
        let mut record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            100,
            100,
            (1..=MAX_AUDITED_RETAINED_GENERATIONS as u64 + 1).collect(),
            digest(),
            false,
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
        let mut record = SecretsLifecycleAuditFields::rolled_back(
            &ctx(LifecycleAction::Rollback),
            1,
            2,
            vec![2],
            false,
        );
        record.material_digest_hex = Some(digest());
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest)
        );
    }

    #[test]
    fn created_and_rotated_require_material_digest() {
        let mut created = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        created.material_digest_hex = None;
        assert_eq!(
            created.validate(),
            Err(SecretsLifecycleAuditError::MissingMaterialDigest)
        );

        let mut rotated = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            2,
            vec![1],
            digest(),
            false,
        );
        rotated.material_digest_hex = None;
        assert_eq!(
            rotated.validate(),
            Err(SecretsLifecycleAuditError::MissingMaterialDigest)
        );
    }

    #[test]
    fn material_digest_must_be_lowercase_hex_64() {
        let mut record = SecretsLifecycleAuditFields::provisioned(
            &ctx(LifecycleAction::Provision),
            1,
            digest(),
            false,
        );
        for bad in ["", "AA", &"a".repeat(63), &"g".repeat(64), &"A".repeat(64)] {
            record.material_digest_hex = Some(bad.to_owned());
            assert_eq!(
                record.validate(),
                Err(SecretsLifecycleAuditError::InvalidMaterialDigest)
            );
        }
    }

    #[test]
    fn action_result_matrix_rejects_impossible_combinations() {
        // `Provision` can never produce `Rotated`.
        let mut impossible = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            2,
            2,
            vec![1],
            digest(),
            false,
        );
        impossible.action = LifecycleAction::Provision;
        assert_eq!(
            impossible.validate(),
            Err(SecretsLifecycleAuditError::ActionResultIncompatible)
        );

        // `Rollback` can never produce `VerifiedClean` (only `Retire`
        // reaches that result).
        let mut impossible2 =
            SecretsLifecycleAuditFields::verified_clean(&ctx(LifecycleAction::Retire), 3);
        impossible2.action = LifecycleAction::Rollback;
        assert_eq!(
            impossible2.validate(),
            Err(SecretsLifecycleAuditError::ActionResultIncompatible)
        );
    }

    #[test]
    fn serde_round_trip_preserves_every_field() {
        let record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            3,
            3,
            vec![2],
            digest(),
            true,
        );
        let json = serde_json::to_string(&record).expect("serialize");
        let decoded: SecretsLifecycleAuditFields =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, record);
        assert!(decoded.recovered_prior_transaction);
    }

    #[test]
    fn fail_reason_serde_round_trips_as_closed_enum() {
        let json = serde_json::to_string(&FailReason::RecoveryContentMismatch).unwrap();
        assert_eq!(json, "\"recovery_content_mismatch\"");
        let decoded: FailReason = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, FailReason::RecoveryContentMismatch);
        // An arbitrary string is not a valid FailReason: this is the
        // structural guarantee behind "closed failure reasons".
        assert!(serde_json::from_str::<FailReason>("\"totally-made-up\"").is_err());
    }

    #[test]
    fn verified_clean_never_carries_lineage_epoch() {
        let record = SecretsLifecycleAuditFields::verified_clean(&ctx(LifecycleAction::Retire), 7);
        record
            .validate()
            .expect("verified clean record must validate");
        assert_eq!(record.lineage_epoch, None);
        assert_eq!(record.high_water_epoch, Some(7));
    }
}
