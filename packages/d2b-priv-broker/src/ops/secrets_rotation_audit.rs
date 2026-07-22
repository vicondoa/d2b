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
//! deliberately never carries raw secret material or a raw
//! channel-binding/credential byte string — only a bounded opaque
//! typed [`WorkloadId`] (never a human VM-name label or a filesystem
//! path; see the W8fu6 note below), a SHA-256 `material_digest_hex`
//! fingerprint (so operators can correlate a rotation to specific
//! delivered material without ever seeing it), and closed-set
//! result/marker/reason enums. **`fail_reason` is a closed enum, not a
//! free-form string** — this is a deliberate hardening over an earlier
//! draft of this module that carried `Option<String>` populated from
//! convention-only `&'static str` constants; nothing outside this
//! file's [`FailReason`] enum can ever reach the audit surface.
//!
//! # W8fu6: canonical typed identity, no filesystem path
//!
//! Rounds 1-5 bound this schema to a bare [`d2b_contracts::types::VmId`]
//! human-label string plus an FNV1a-64 `base_dir_hash` of the
//! filesystem path the old engine derived from it. The W8fu6 rewrite of
//! [`crate::ops::secrets_lifecycle`] into a pure transaction core over
//! an injected, storage-agnostic authority port has no filesystem path
//! at all, and this schema now binds to the canonical
//! [`d2b_contracts::v2_identity::WorkloadId`] typed identity instead of
//! a legacy VM-name string: `base_dir_hash` is removed outright (there
//! is nothing left to hash), and `vm_id` is renamed `workload_id` with
//! the new type. `WorkloadId`'s wire form is still a plain bounded
//! opaque 20-character string (never the human-readable workload
//! name/label it was derived from), so this remains schema-safe for any
//! JSON consumer that only ever treats the field as an opaque
//! correlation token.
//!
//! # Status
//!
//! This schema, and the [`crate::ops::secrets_lifecycle`] engine that
//! produces it, are **not wired into any live broker dispatch path or
//! `crate::audit::AuditLog` sink**. See that module's "Integration
//! wiring points" doc section for the exact list of follow-up steps a
//! future integrator must perform before any of this is observable in
//! a running broker's audit log.

use d2b_contracts::v2_identity::WorkloadId;
use serde::{Deserialize, Serialize};

/// Schema version for the secrets-lifecycle terminal audit record.
///
/// Bumped from `2` to `3` for the W8fu6 ports-and-adapters rewrite:
/// `vm_id: VmId` became `workload_id: WorkloadId` (the canonical v2
/// identity type, replacing a legacy human VM-name string);
/// `base_dir_hash` was removed outright (the new engine has no
/// filesystem path to hash); `recovered_prior_transaction` was removed
/// (the pure CAS-based engine has no separate "recovery" phase — a
/// fenced writer simply retries, it never "recovers" a crashed
/// transaction left by itself); and a new `prune_deferred: bool` field
/// was added, surfacing the new engine's prune-after-commit-debt model
/// on the audit surface.
pub const SECRETS_LIFECYCLE_AUDIT_SCHEMA_VERSION: u32 = 3;

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
    /// Stable slug used as the audit-record `kind` discriminant's
    /// string form for any downstream consumer that only speaks JSON.
    /// (Rounds 1-5 also used this as an on-disk directory component;
    /// the W8fu6 storage-agnostic engine has no directory, so this is
    /// now purely a wire/audit-surface identifier.)
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
    /// A new generation was created and became the active generation.
    Rotated,
    /// The active generation was atomically swapped back to the
    /// retained previous generation.
    RolledBack,
    /// Every generation was durably retired.
    Retired,
    /// The requested action was already satisfied (e.g. `retire` on an
    /// already-retired or never-provisioned kind); no mutation
    /// occurred.
    VerifiedClean,
    /// The request was refused before any mutation (e.g. `rotate`
    /// requested for a never-provisioned kind).
    Denied,
    /// The step aborted partway and failed closed. Never a silent
    /// partial mutation: [`LifecycleResult::FailedClosed`] is only ever
    /// reachable before a commit durably succeeds, or after a
    /// post-commit integrity re-check detects tampering and
    /// quarantines the authority — never as a way to paper over an
    /// already-durable, already-activated state.
    FailedClosed,
}

/// Terminal disposition of the identity-binding integrity check for
/// this action. Rounds 1-5 named this after the on-disk marker file
/// that recorded a generation's trusted identity; the W8fu6 rewrite
/// keeps the type name and variant set (per the explicit instruction
/// to retain closed audit/channel types across the rewrite) but the
/// meaning generalizes to "the terminal state of this action's
/// identity-binding checks against the authority port", independent of
/// whatever concrete marker/metadata representation an adapter uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerResult {
    /// A fresh generation's identity was durably committed for the
    /// first time.
    Created,
    /// An existing generation's live digest was independently
    /// re-verified against the durably committed identity before this
    /// action mutated anything.
    Verified,
    /// The durable state was rewritten to record a completed
    /// retirement.
    Tombstoned,
    /// The action never reached a mutation step (denied before any
    /// authority-port write).
    Unchanged,
    /// The durable state failed a self-consistency or digest
    /// verification check, or a commit/prune step failed. The action
    /// fails closed.
    FailedClosed,
}

/// Closed set of redaction-safe failure/refusal reasons. Every
/// [`SecretsLifecycleAuditFields::denied`] /
/// [`SecretsLifecycleAuditFields::failed`] call site in
/// `secrets_lifecycle.rs` constructs one of these variants directly —
/// there is no code path that can place an arbitrary string, a raw
/// filesystem path, or an adapter-internal error detail onto the audit
/// surface.
///
/// This is a substantially smaller, storage-agnostic set than the
/// rounds 1-5 filesystem-anchored engine's 27-variant enum: every
/// variant naming a raw path/lock/fsync/txlog/ACL/inode/link-count
/// concept was a property of that engine's *own* filesystem adapter,
/// never observable by the W8fu6 pure transaction core, which only ever
/// calls the six guarded, typed [`crate::ops::secrets_lifecycle::SecretsAuthorityPort`]
/// methods. All of that fine-grained tamper/I/O detail is now the
/// adapter's own internal concern, hidden behind
/// [`crate::ops::secrets_lifecycle::PortError`] and reported to this
/// audit surface, when it must be, only via the single opaque
/// [`FailReason::PortUnavailable`] bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// `provision` requested for a `(workload, kind)` that already has
    /// an active, non-retired generation.
    AlreadyProvisioned,
    /// `rotate`/`rollback` requested for a `(workload, kind)` with no
    /// active generation (never provisioned, or already retired).
    NotProvisioned,
    /// `rollback` requested with no retained prior generation to swap
    /// back to.
    NoRollbackTarget,
    /// The caller-supplied material was empty or exceeded
    /// [`crate::ops::secrets_lifecycle::SecretMaterial::MAX_LEN`].
    /// Never actually reachable as an audit record's `fail_reason`:
    /// [`crate::ops::secrets_lifecycle::SecretMaterial::new`] returns
    /// this directly to the caller before any
    /// [`SecretsLifecycleAuditContext`] exists. Kept in this enum
    /// anyway so every lifecycle failure a caller must handle shares
    /// one closed vocabulary.
    InvalidMaterial,
    /// A generation's independently recomputed live digest did not
    /// match the durable state's recorded digest for that generation
    /// — live storage was tampered with, or (post-commit) a concurrent
    /// writer's later stage attempt clobbered this call's own
    /// just-committed material. The authority is quarantined before
    /// this is returned.
    ChecksumMismatch,
    /// [`crate::ops::secrets_lifecycle::SecretsAuthorityPort::cas_commit`]
    /// reported that another writer's transition committed first.
    /// Nothing was mutated; the caller may re-read and retry.
    OwnershipFenced,
    /// The authority already reported (or newly detected and
    /// self-reported) that this `(workload, kind)` is quarantined.
    /// Every port call for this pair fails closed until an
    /// out-of-band clearing operation this module does not expose
    /// resets it.
    Quarantined,
    /// The durable state read from the authority failed its own
    /// internal self-consistency check (see
    /// [`crate::ops::secrets_lifecycle::DurableState::validate_self_consistent`]).
    /// The authority is quarantined before this is returned.
    StateCorrupt,
    /// A prior action's pending prune obligation could not be fully
    /// resolved before this action could proceed. Whatever subset was
    /// resolvable was durably committed; the caller may retry.
    PruneDebtUnresolved,
    /// The authority reported an error internal to its own adapter
    /// (I/O, its own locking/storage substrate, etc). This module
    /// never inspects or forwards the adapter's own error detail —
    /// only this one opaque, closed bucket ever reaches the audit
    /// surface.
    PortUnavailable,
}

impl FailReason {
    /// Stable slug matching the enum variant's `snake_case` wire form,
    /// safe for any Debug/log/audit surface (never a path or secret).
    pub fn as_slug(self) -> &'static str {
        match self {
            Self::AlreadyProvisioned => "already_provisioned",
            Self::NotProvisioned => "not_provisioned",
            Self::NoRollbackTarget => "no_rollback_target",
            Self::InvalidMaterial => "invalid_material",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::OwnershipFenced => "ownership_fenced",
            Self::Quarantined => "quarantined",
            Self::StateCorrupt => "state_corrupt",
            Self::PruneDebtUnresolved => "prune_debt_unresolved",
            Self::PortUnavailable => "port_unavailable",
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
    /// Canonical typed, opaque workload identity — never a
    /// human-readable VM-name label or a filesystem path. See the
    /// module-level "W8fu6" doc section.
    pub workload_id: WorkloadId,
    pub kind: SecretKind,
    pub action: LifecycleAction,
}

/// Path-free, material-free terminal audit record for one secrets
/// lifecycle attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretsLifecycleAuditFields {
    pub schema_version: u32,
    /// Canonical typed, opaque workload identity — see
    /// [`SecretsLifecycleAuditContext::workload_id`].
    pub workload_id: WorkloadId,
    pub kind: SecretKind,
    pub action: LifecycleAction,
    pub result: LifecycleResult,
    pub marker_result: MarkerResult,
    /// The active lineage epoch after this action, when one exists
    /// (`None` after a successful `retire`, and for `Denied`/
    /// `FailedClosed` results). This is the monotonic identity anchor
    /// (see `secrets_lifecycle::DurableState::high_water_epoch`) —
    /// never simply "current epoch number + 1", so a rotate issued
    /// after a rollback can never collide with (or silently resurrect)
    /// a still-materialized older epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_epoch: Option<u64>,
    /// The monotonic high-water epoch recorded by the durable state
    /// after this action (present exactly when `lineage_epoch` is
    /// present, and additionally present after a successful `retire`
    /// so an auditor can confirm a subsequent re-provision's first
    /// epoch is still strictly greater than every epoch this
    /// `(workload, kind)` ever used). Never decreases across the
    /// lifetime of a `(workload, kind)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_water_epoch: Option<u64>,
    /// Generations retained for rollback after this action (excludes
    /// the active generation).
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
    /// `true` when this action's own successful commit left at least
    /// one superseded generation not yet synchronously pruned (the
    /// authority durably records the debt in
    /// `secrets_lifecycle::DurableState::pending_prune`; a future
    /// action for this `(workload, kind)` resolves it before doing its
    /// own work). Only reachable when `result` is `Rotated` or
    /// `Retired` — every other result either wrote no new pending-prune
    /// debt (`Created`, `RolledBack`) or performed no mutation at all
    /// (`VerifiedClean`, `Denied`, `FailedClosed`).
    #[serde(default)]
    pub prune_deferred: bool,
}

/// Errors [`SecretsLifecycleAuditFields::validate`] can report. Kept
/// separate from [`crate::ops::OpError`] because this module has no
/// dependency on the rest of `ops::` beyond `serde`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsLifecycleAuditError {
    SchemaVersionMismatch { found: u32 },
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
    UnexpectedPruneDeferred,
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
            Self::UnexpectedPruneDeferred => write!(
                f,
                "prune_deferred is only reachable for Rotated/Retired results"
            ),
        }
    }
}

impl std::error::Error for SecretsLifecycleAuditError {}

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
        if self.prune_deferred
            && !matches!(
                self.result,
                LifecycleResult::Rotated | LifecycleResult::Retired
            )
        {
            return Err(SecretsLifecycleAuditError::UnexpectedPruneDeferred);
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
            workload_id: ctx.workload_id.clone(),
            kind: ctx.kind,
            action: ctx.action,
            result: LifecycleResult::VerifiedClean,
            marker_result: MarkerResult::Unchanged,
            lineage_epoch: None,
            high_water_epoch: None,
            retained_generations: Vec::new(),
            material_digest_hex: None,
            fail_reason: None,
            prune_deferred: false,
        }
    }

    /// A fresh generation 1 was provisioned.
    pub fn provisioned(
        ctx: &SecretsLifecycleAuditContext,
        high_water_epoch: u64,
        material_digest_hex: String,
    ) -> Self {
        Self {
            result: LifecycleResult::Created,
            marker_result: MarkerResult::Created,
            lineage_epoch: Some(1),
            high_water_epoch: Some(high_water_epoch),
            material_digest_hex: Some(material_digest_hex),
            ..Self::base(ctx)
        }
    }

    /// A new generation was created and activated.
    pub fn rotated(
        ctx: &SecretsLifecycleAuditContext,
        lineage_epoch: u64,
        high_water_epoch: u64,
        retained_generations: Vec<u64>,
        material_digest_hex: String,
        prune_deferred: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::Rotated,
            marker_result: MarkerResult::Verified,
            lineage_epoch: Some(lineage_epoch),
            high_water_epoch: Some(high_water_epoch),
            retained_generations,
            material_digest_hex: Some(material_digest_hex),
            prune_deferred,
            ..Self::base(ctx)
        }
    }

    /// The active generation was swapped back to a retained prior
    /// generation. Never defers a prune (a rollback supersedes
    /// nothing new; see `secrets_lifecycle::rollback`'s doc comment).
    pub fn rolled_back(
        ctx: &SecretsLifecycleAuditContext,
        lineage_epoch: u64,
        high_water_epoch: u64,
        retained_generations: Vec<u64>,
    ) -> Self {
        Self {
            result: LifecycleResult::RolledBack,
            marker_result: MarkerResult::Verified,
            lineage_epoch: Some(lineage_epoch),
            high_water_epoch: Some(high_water_epoch),
            retained_generations,
            ..Self::base(ctx)
        }
    }

    /// Every generation was retired.
    pub fn retired(
        ctx: &SecretsLifecycleAuditContext,
        high_water_epoch: u64,
        prune_deferred: bool,
    ) -> Self {
        Self {
            result: LifecycleResult::Retired,
            marker_result: MarkerResult::Tombstoned,
            high_water_epoch: Some(high_water_epoch),
            prune_deferred,
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

    /// The action was refused before any authority-port mutation.
    pub fn denied(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> Self {
        Self {
            result: LifecycleResult::Denied,
            marker_result: MarkerResult::Unchanged,
            fail_reason: Some(reason),
            ..Self::base(ctx)
        }
    }

    /// The action aborted partway and failed closed.
    ///
    /// `marker_result` is deliberately **not** a caller-supplied
    /// parameter: every reachable `secrets_lifecycle.rs` call site
    /// already only ever passes `MarkerResult::FailedClosed` (this
    /// is the *only* result variant `validate` accepts for
    /// `FailedClosed`, see the match arm above), so accepting an
    /// arbitrary `MarkerResult` here was dead flexibility that could
    /// only ever construct an invalid, immediately-`validate`-
    /// rejected record. Hardcoding it makes that invalid state
    /// unconstructible instead of merely unreachable-in-practice.
    pub fn failed(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> Self {
        Self {
            result: LifecycleResult::FailedClosed,
            marker_result: MarkerResult::FailedClosed,
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
            workload_id: WorkloadId::parse("aaaaaaaaaaaaaaaaaaaa").expect("valid fixture id"),
            kind: SecretKind::GuestSigningKey,
            action,
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
            FailReason::AlreadyProvisioned,
            FailReason::NotProvisioned,
            FailReason::NoRollbackTarget,
            FailReason::InvalidMaterial,
            FailReason::ChecksumMismatch,
            FailReason::OwnershipFenced,
            FailReason::Quarantined,
            FailReason::StateCorrupt,
            FailReason::PruneDebtUnresolved,
            FailReason::PortUnavailable,
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
        let record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
        record.validate().expect("provisioned record must validate");
        assert_eq!(record.lineage_epoch, Some(1));
        assert_eq!(record.high_water_epoch, Some(1));
        assert!(record.retained_generations.is_empty());
        assert!(!record.prune_deferred);
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
    fn rotated_may_defer_a_prune() {
        let record = SecretsLifecycleAuditFields::rotated(
            &ctx(LifecycleAction::Rotate),
            3,
            3,
            vec![2],
            digest(),
            true,
        );
        record
            .validate()
            .expect("rotated record with prune_deferred must validate");
        assert!(record.prune_deferred);
    }

    #[test]
    fn prune_deferred_is_rejected_outside_rotated_and_retired() {
        let mut record = SecretsLifecycleAuditFields::rolled_back(
            &ctx(LifecycleAction::Rollback),
            1,
            2,
            vec![2],
        );
        record.prune_deferred = true;
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedPruneDeferred)
        );

        let mut denied = SecretsLifecycleAuditFields::denied(
            &ctx(LifecycleAction::Rotate),
            FailReason::NotProvisioned,
        );
        denied.prune_deferred = true;
        assert_eq!(
            denied.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedPruneDeferred)
        );
    }

    #[test]
    fn rolled_back_validates_and_requires_retained_generation() {
        let record = SecretsLifecycleAuditFields::rolled_back(
            &ctx(LifecycleAction::Rollback),
            1,
            2,
            vec![2],
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
    fn retired_may_defer_a_prune() {
        let record = SecretsLifecycleAuditFields::retired(&ctx(LifecycleAction::Retire), 5, true);
        record
            .validate()
            .expect("retired record with prune_deferred must validate");
        assert!(record.prune_deferred);
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
            FailReason::StateCorrupt,
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
            FailReason::ChecksumMismatch,
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
            FailReason::OwnershipFenced,
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

    /// `retire` + `Denied` is a genuinely reachable pair (a prune-debt
    /// refusal denies a retire attempt exactly like every other
    /// action) — the action/result matrix above must accept it, not
    /// just the three previously-listed retire outcomes.
    #[test]
    fn retire_denied_is_a_reachable_and_valid_pair() {
        let record = SecretsLifecycleAuditFields::denied(
            &ctx(LifecycleAction::Retire),
            FailReason::PruneDebtUnresolved,
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
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
        record.schema_version = 2;
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::SchemaVersionMismatch { found: 2 })
        );
    }

    #[test]
    fn lineage_epoch_zero_is_rejected() {
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
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
        );
        record.material_digest_hex = Some(digest());
        assert_eq!(
            record.validate(),
            Err(SecretsLifecycleAuditError::UnexpectedMaterialDigest)
        );
    }

    #[test]
    fn created_and_rotated_require_material_digest() {
        let mut created =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
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
        let mut record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
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
        assert!(decoded.prune_deferred);
    }

    #[test]
    fn fail_reason_serde_round_trips_as_closed_enum() {
        let json = serde_json::to_string(&FailReason::ChecksumMismatch).unwrap();
        assert_eq!(json, "\"checksum_mismatch\"");
        let decoded: FailReason = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, FailReason::ChecksumMismatch);
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

    /// The wire form of `workload_id` stays a bounded opaque 20-byte
    /// string (never the human-readable workload name it was derived
    /// from), so a downstream consumer that only speaks JSON still
    /// sees a plain string field, not a nested object.
    #[test]
    fn workload_id_serializes_as_a_plain_opaque_string() {
        let record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
        let json = serde_json::to_value(&record).expect("serialize");
        assert_eq!(
            json.get("workloadId").and_then(|v| v.as_str()),
            Some("aaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let record =
            SecretsLifecycleAuditFields::provisioned(&ctx(LifecycleAction::Provision), 1, digest());
        let mut value = serde_json::to_value(&record).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .insert("baseDirHash".to_owned(), serde_json::json!("deadbeef"));
        let decoded: Result<SecretsLifecycleAuditFields, _> = serde_json::from_value(value);
        assert!(
            decoded.is_err(),
            "a legacy base_dir_hash field must be rejected by deny_unknown_fields"
        );
    }
}
