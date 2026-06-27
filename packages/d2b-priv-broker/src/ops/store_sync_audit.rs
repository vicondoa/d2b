//! Signed `StoreSync` terminal audit schema (ADR 0027).
//!
//! Every `StoreSync` attempt emits exactly one terminal structured
//! broker audit record. This module is the typed shape of that record's
//! `operation_fields` object plus the constructors that make invalid
//! records unrepresentable for the call-sites, and a [`validate`] pass
//! that the JSON drift / schema tests use to reject hand-built invalid
//! combinations.
//!
//! Scope note (W4): the constructors model the full signed enum surface.
//! The success path ([`ok_fast_path`](StoreSyncAuditFields::ok_fast_path),
//! [`ok_non_fast_path`](StoreSyncAuditFields::ok_non_fast_path)) and the
//! failure path ([`failed`](StoreSyncAuditFields::failed)) are wired into
//! dispatch: every `run_store_sync` attempt that reaches the handler now
//! emits exactly one terminal record, success or failure, with a
//! classified `error_stage`. The `ok_cleanup_failed` shape is reachable
//! from the post-activation cleanup path. The remaining deferred shape is
//! [`denied`](StoreSyncAuditFields::denied): it awaits a per-VM/per-caller
//! StoreSync authorization policy (the only kernel-trusted identity at this
//! layer is the global peer-uid gate applied before dispatch). Successful
//! StoreSync attempts populate available per-phase timings; failure paths
//! still carry the dispatch-level `total_ms`.
//! See `docs/reference/store-sync.md`.
//!
//! Redaction contract: this is the **host-confidential** audit record
//! (broker audit log is `0640 root:d2bd`). It deliberately does NOT
//! carry store-path basenames, `db.dump`/marker payloads, or
//! host-absolute symlink targets. `caller_principal` and
//! `retained_generations` are audit-only and must never be re-exported to
//! the guest `meta.json` or to any future StoreSync observability export
//! (a separate positive-allow-list serializer owns that surface).

use serde::{Deserialize, Serialize};

/// Schema version for the `StoreSync` terminal audit record.
pub const STORE_SYNC_AUDIT_SCHEMA_VERSION: u32 = 1;

/// Terminal sync status. `in_progress` is a transient pre-terminal state
/// only; terminal records are `ok` or `failed` (a terminal attempt metric
/// for `sync_status` is never `in_progress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Ok,
    Failed,
    InProgress,
}

/// The boundary a failing sync stopped at. `none` for successful syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStage {
    None,
    Authz,
    Lock,
    Probe,
    Verify,
    Stage,
    Rename,
    Metadata,
    Integrity,
    CurrentSwap,
    Marker,
}

/// Post-activation cleanup/sweep disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    NotAttempted,
    Completed,
    DeferredOnline,
    DeferredAmbiguous,
    DeferredMetadata,
    SkippedFastPath,
    Failed,
}

/// Why cleanup landed in its [`CleanupStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupReason {
    None,
    VmRunning,
    RunningGenerationAmbiguous,
    MissingRetainedMetadata,
    IoError,
    FastPath,
}

/// Authorization decision for the caller against the target VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthzOutcome {
    Allow,
    Deny,
}

/// Per-phase millisecond timings. Phases the current execution path does
/// not yet measure are reported as `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSyncTimings {
    pub total_ms: u64,
    pub lock_wait_ms: u64,
    pub lock_hold_ms: u64,
    pub probe_ms: u64,
    pub verify_ms: u64,
    pub stage_ms: u64,
    pub metadata_ms: u64,
    pub sweep_ms: u64,
    pub cleanup_ms: u64,
}

/// Always-present context for a `StoreSync` terminal audit record. The
/// constructors thread this through so the per-status variant only has to
/// supply the fields that vary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSyncAuditContext {
    pub vm: String,
    pub vm_id: String,
    /// Host-audit field; never exposed to guest metadata. Optional because
    /// env-less/system VMs may not have a manifest env.
    pub env: Option<String>,
    /// Opaque bundle intent ref (host-only context, not a store path).
    pub bundle_closure_ref: String,
    /// Per-VM store-view root (host path; never a store-path basename).
    pub hardlink_farm_path: String,
    pub generation_id: String,
    pub generation_token: u32,
    /// Audit-only caller identity string (kernel peer credentials). Never
    /// a metric label and never re-exported to the guest.
    pub caller_principal: Option<String>,
    pub closure_count: u32,
    pub timings: StoreSyncTimings,
}

/// Typed `operation_fields` body for the `StoreSync` audit record.
///
/// Invariants are enforced by the constructors and re-checkable via
/// [`StoreSyncAuditFields::validate`]. The field set is the ADR 0027
/// signed schema plus the pre-existing host-only context fields
/// (`bundle_closure_ref`, `hardlink_farm_path`), which stay in the
/// host-confidential record only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSyncAuditFields {
    pub schema_version: u32,
    pub vm: String,
    pub vm_id: String,
    pub env: Option<String>,
    pub bundle_closure_ref: String,
    pub hardlink_farm_path: String,
    pub generation_id: String,
    pub generation_token: u32,
    pub sync_status: SyncStatus,
    pub error_stage: ErrorStage,
    pub cleanup_status: CleanupStatus,
    pub cleanup_reason: CleanupReason,
    pub caller_principal: Option<String>,
    pub authz_outcome: AuthzOutcome,
    pub closure_count: u32,
    pub linked_count: u32,
    pub skipped_count: u32,
    pub retained_generations: Vec<u32>,
    pub swept_count: u32,
    pub fast_path: bool,
    pub timings: StoreSyncTimings,
}

/// Reasons a [`StoreSyncAuditFields`] record violates the signed schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreSyncAuditError {
    /// `sync_status = ok` requires `error_stage = none`.
    OkWithErrorStage(ErrorStage),
    /// `(cleanup_status, cleanup_reason)` is not one of the valid pairs.
    InvalidCleanupPair(CleanupStatus, CleanupReason),
    /// A `failed` terminal record must use `cleanup_status = not_attempted`,
    /// `cleanup_reason = none`, and a concrete (non-`none`) `error_stage`.
    FailedShape {
        error_stage: ErrorStage,
        cleanup_status: CleanupStatus,
        cleanup_reason: CleanupReason,
    },
    /// `error_stage = authz` iff `authz_outcome = deny`.
    AuthzStageOutcomeMismatch {
        error_stage: ErrorStage,
        authz_outcome: AuthzOutcome,
    },
    /// For `ok` records (other than a post-activation cleanup failure),
    /// `linked_count + skipped_count` must equal `closure_count`.
    OkAccountingMismatch {
        linked_count: u32,
        skipped_count: u32,
        closure_count: u32,
    },
    /// A `fast_path` record must report `linked_count = 0` and
    /// `skipped_count = closure_count`.
    FastPathAccounting {
        linked_count: u32,
        skipped_count: u32,
        closure_count: u32,
    },
    /// `cleanup_status = skipped_fast_path` requires `fast_path = true` and
    /// `swept_count = 0`.
    SkippedFastPathShape { fast_path: bool, swept_count: u32 },
    /// `in_progress` is transient: nothing terminal may have happened
    /// (`error_stage = none`, `cleanup_status = not_attempted`,
    /// `cleanup_reason = none`).
    InProgressShape {
        error_stage: ErrorStage,
        cleanup_status: CleanupStatus,
        cleanup_reason: CleanupReason,
    },
}

impl std::fmt::Display for StoreSyncAuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OkWithErrorStage(stage) => {
                write!(f, "sync_status=ok requires error_stage=none, got {stage:?}")
            }
            Self::InvalidCleanupPair(status, reason) => write!(
                f,
                "invalid (cleanup_status, cleanup_reason) pair: ({status:?}, {reason:?})"
            ),
            Self::FailedShape {
                error_stage,
                cleanup_status,
                cleanup_reason,
            } => write!(
                f,
                "failed record must be (error_stage!=none, not_attempted, none); got \
                 ({error_stage:?}, {cleanup_status:?}, {cleanup_reason:?})"
            ),
            Self::AuthzStageOutcomeMismatch {
                error_stage,
                authz_outcome,
            } => write!(
                f,
                "error_stage=authz iff authz_outcome=deny; got ({error_stage:?}, {authz_outcome:?})"
            ),
            Self::OkAccountingMismatch {
                linked_count,
                skipped_count,
                closure_count,
            } => write!(
                f,
                "ok record requires linked+skipped==closure_count; got {linked_count}+{skipped_count}!={closure_count}"
            ),
            Self::FastPathAccounting {
                linked_count,
                skipped_count,
                closure_count,
            } => write!(
                f,
                "fast_path record requires linked=0 & skipped=closure_count; got linked={linked_count} skipped={skipped_count} closure={closure_count}"
            ),
            Self::SkippedFastPathShape {
                fast_path,
                swept_count,
            } => write!(
                f,
                "skipped_fast_path requires fast_path=true & swept_count=0; got fast_path={fast_path} swept={swept_count}"
            ),
            Self::InProgressShape {
                error_stage,
                cleanup_status,
                cleanup_reason,
            } => write!(
                f,
                "in_progress must be (none, not_attempted, none); got \
                 ({error_stage:?}, {cleanup_status:?}, {cleanup_reason:?})"
            ),
        }
    }
}

impl std::error::Error for StoreSyncAuditError {}

/// The exact set of valid `(cleanup_status, cleanup_reason)` pairs
/// (ADR 0027). Anything else is rejected.
fn cleanup_pair_is_valid(status: CleanupStatus, reason: CleanupReason) -> bool {
    matches!(
        (status, reason),
        (CleanupStatus::Completed, CleanupReason::None)
            | (CleanupStatus::NotAttempted, CleanupReason::None)
            | (CleanupStatus::DeferredOnline, CleanupReason::VmRunning)
            | (
                CleanupStatus::DeferredAmbiguous,
                CleanupReason::RunningGenerationAmbiguous
            )
            | (
                CleanupStatus::DeferredMetadata,
                CleanupReason::MissingRetainedMetadata
            )
            | (CleanupStatus::SkippedFastPath, CleanupReason::FastPath)
            | (CleanupStatus::Failed, CleanupReason::IoError)
    )
}

impl StoreSyncAuditFields {
    /// Re-check every signed schema invariant. Correct-by-construction
    /// constructors always produce records that pass; the JSON drift /
    /// schema tests use this to reject hand-built invalid combinations.
    pub fn validate(&self) -> Result<(), StoreSyncAuditError> {
        // Cleanup pair must be one of the allow-listed combinations.
        if !cleanup_pair_is_valid(self.cleanup_status, self.cleanup_reason) {
            return Err(StoreSyncAuditError::InvalidCleanupPair(
                self.cleanup_status,
                self.cleanup_reason,
            ));
        }

        // error_stage = authz iff authz_outcome = deny.
        let is_authz_stage = matches!(self.error_stage, ErrorStage::Authz);
        let is_deny = matches!(self.authz_outcome, AuthzOutcome::Deny);
        if is_authz_stage != is_deny {
            return Err(StoreSyncAuditError::AuthzStageOutcomeMismatch {
                error_stage: self.error_stage,
                authz_outcome: self.authz_outcome,
            });
        }

        // skipped_fast_path shape (the pair already constrains the reason).
        if matches!(self.cleanup_status, CleanupStatus::SkippedFastPath)
            && (!self.fast_path || self.swept_count != 0)
        {
            return Err(StoreSyncAuditError::SkippedFastPathShape {
                fast_path: self.fast_path,
                swept_count: self.swept_count,
            });
        }

        match self.sync_status {
            SyncStatus::Ok => {
                if !matches!(self.error_stage, ErrorStage::None) {
                    return Err(StoreSyncAuditError::OkWithErrorStage(self.error_stage));
                }
                // A post-activation cleanup failure is the only ok record
                // exempt from the linked+skipped accounting equality
                // (activation succeeded, but the count may be incomplete).
                let cleanup_failed = matches!(self.cleanup_status, CleanupStatus::Failed);
                if !cleanup_failed && self.linked_count + self.skipped_count != self.closure_count {
                    return Err(StoreSyncAuditError::OkAccountingMismatch {
                        linked_count: self.linked_count,
                        skipped_count: self.skipped_count,
                        closure_count: self.closure_count,
                    });
                }
                // A fast path never relinks: linked=0, skipped=closure_count.
                if self.fast_path
                    && (self.linked_count != 0 || self.skipped_count != self.closure_count)
                {
                    return Err(StoreSyncAuditError::FastPathAccounting {
                        linked_count: self.linked_count,
                        skipped_count: self.skipped_count,
                        closure_count: self.closure_count,
                    });
                }
                Ok(())
            }
            SyncStatus::Failed => {
                // Failed-before-cleanup: a concrete stage, no cleanup.
                let ok_shape = !matches!(self.error_stage, ErrorStage::None)
                    && matches!(self.cleanup_status, CleanupStatus::NotAttempted)
                    && matches!(self.cleanup_reason, CleanupReason::None);
                if !ok_shape {
                    return Err(StoreSyncAuditError::FailedShape {
                        error_stage: self.error_stage,
                        cleanup_status: self.cleanup_status,
                        cleanup_reason: self.cleanup_reason,
                    });
                }
                Ok(())
            }
            SyncStatus::InProgress => {
                if !matches!(self.error_stage, ErrorStage::None)
                    || !matches!(self.cleanup_status, CleanupStatus::NotAttempted)
                    || !matches!(self.cleanup_reason, CleanupReason::None)
                {
                    return Err(StoreSyncAuditError::InProgressShape {
                        error_stage: self.error_stage,
                        cleanup_status: self.cleanup_status,
                        cleanup_reason: self.cleanup_reason,
                    });
                }
                Ok(())
            }
        }
    }

    fn base(ctx: StoreSyncAuditContext) -> Self {
        Self {
            schema_version: STORE_SYNC_AUDIT_SCHEMA_VERSION,
            vm: ctx.vm,
            vm_id: ctx.vm_id,
            env: ctx.env,
            bundle_closure_ref: ctx.bundle_closure_ref,
            hardlink_farm_path: ctx.hardlink_farm_path,
            generation_id: ctx.generation_id,
            generation_token: ctx.generation_token,
            sync_status: SyncStatus::Ok,
            error_stage: ErrorStage::None,
            cleanup_status: CleanupStatus::NotAttempted,
            cleanup_reason: CleanupReason::None,
            caller_principal: ctx.caller_principal,
            authz_outcome: AuthzOutcome::Allow,
            closure_count: ctx.closure_count,
            linked_count: 0,
            skipped_count: 0,
            retained_generations: Vec::new(),
            swept_count: 0,
            fast_path: false,
            timings: ctx.timings,
        }
    }

    /// Successful non-fast-path materialisation whose post-activation
    /// cleanup is deferred because the running generation cannot yet be
    /// determined (the running-generation retention detector is a
    /// follow-up wave). Maps to `deferred_ambiguous` +
    /// `running_generation_ambiguous`.
    pub fn ok_non_fast_path(
        ctx: StoreSyncAuditContext,
        linked_count: u32,
        skipped_count: u32,
        retained_generations: Vec<u32>,
    ) -> Self {
        Self {
            sync_status: SyncStatus::Ok,
            cleanup_status: CleanupStatus::DeferredAmbiguous,
            cleanup_reason: CleanupReason::RunningGenerationAmbiguous,
            linked_count,
            skipped_count,
            retained_generations,
            fast_path: false,
            ..Self::base(ctx)
        }
    }

    pub fn ok_non_fast_path_with_cleanup(
        ctx: StoreSyncAuditContext,
        linked_count: u32,
        skipped_count: u32,
        retained_generations: Vec<u32>,
        swept_count: u32,
        cleanup_status: CleanupStatus,
        cleanup_reason: CleanupReason,
    ) -> Self {
        Self {
            sync_status: SyncStatus::Ok,
            cleanup_status,
            cleanup_reason,
            linked_count,
            skipped_count,
            retained_generations,
            swept_count,
            fast_path: false,
            ..Self::base(ctx)
        }
    }

    /// Pure fast path: a complete, consistent same-generation layout was
    /// already published, so nothing relinked and no sweep ran.
    pub fn ok_fast_path(ctx: StoreSyncAuditContext, retained_generations: Vec<u32>) -> Self {
        Self::ok_fast_path_with_cleanup(
            ctx,
            retained_generations,
            0,
            CleanupStatus::SkippedFastPath,
            CleanupReason::FastPath,
        )
    }

    pub fn ok_fast_path_with_cleanup(
        ctx: StoreSyncAuditContext,
        retained_generations: Vec<u32>,
        swept_count: u32,
        cleanup_status: CleanupStatus,
        cleanup_reason: CleanupReason,
    ) -> Self {
        let closure_count = ctx.closure_count;
        Self {
            sync_status: SyncStatus::Ok,
            cleanup_status,
            cleanup_reason,
            linked_count: 0,
            skipped_count: closure_count,
            retained_generations,
            swept_count,
            fast_path: true,
            ..Self::base(ctx)
        }
    }

    /// Post-activation cleanup failure: the generation activated
    /// successfully (currents + marker committed) but the subsequent
    /// sweep/gcroots step hit an I/O error. Activation is not failed.
    pub fn ok_cleanup_failed(
        ctx: StoreSyncAuditContext,
        linked_count: u32,
        skipped_count: u32,
        retained_generations: Vec<u32>,
        swept_count: u32,
        fast_path: bool,
    ) -> Self {
        Self {
            sync_status: SyncStatus::Ok,
            cleanup_status: CleanupStatus::Failed,
            cleanup_reason: CleanupReason::IoError,
            linked_count,
            skipped_count,
            retained_generations,
            swept_count,
            fast_path,
            ..Self::base(ctx)
        }
    }

    /// Failure before the cleanup/sweep phase at a concrete `error_stage`.
    /// Cleanup never ran (`not_attempted` + `none`).
    pub fn failed(ctx: StoreSyncAuditContext, error_stage: ErrorStage) -> Self {
        // `authz` is modelled separately via [`denied`]; a generic failure
        // at the authz stage without a deny outcome is not representable.
        let error_stage = match error_stage {
            ErrorStage::None | ErrorStage::Authz => ErrorStage::Lock,
            other => other,
        };
        Self {
            sync_status: SyncStatus::Failed,
            error_stage,
            cleanup_status: CleanupStatus::NotAttempted,
            cleanup_reason: CleanupReason::None,
            fast_path: false,
            ..Self::base(ctx)
        }
    }

    /// Authorization denial: refused before lock acquisition or any
    /// filesystem side effect. `error_stage = authz`, `authz_outcome = deny`.
    pub fn denied(ctx: StoreSyncAuditContext) -> Self {
        Self {
            sync_status: SyncStatus::Failed,
            error_stage: ErrorStage::Authz,
            authz_outcome: AuthzOutcome::Deny,
            cleanup_status: CleanupStatus::NotAttempted,
            cleanup_reason: CleanupReason::None,
            fast_path: false,
            ..Self::base(ctx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StoreSyncAuditContext {
        StoreSyncAuditContext {
            vm: "corp-vm".to_owned(),
            vm_id: "store-view:vm:corp-vm".to_owned(),
            env: Some("work".to_owned()),
            bundle_closure_ref: "store-view:vm:corp-vm".to_owned(),
            hardlink_farm_path: "/var/lib/d2b/vms/corp-vm/store-view".to_owned(),
            generation_id: "g-deadbeef".to_owned(),
            generation_token: 42,
            caller_principal: Some("uid:998/role:daemon".to_owned()),
            closure_count: 17,
            timings: StoreSyncTimings {
                total_ms: 12,
                ..StoreSyncTimings::default()
            },
        }
    }

    #[test]
    fn ok_fast_path_has_pure_fast_path_shape() {
        let rec = StoreSyncAuditFields::ok_fast_path(ctx(), vec![42]);
        rec.validate().expect("fast path record is valid");
        assert!(rec.fast_path);
        assert_eq!(rec.linked_count, 0);
        assert_eq!(rec.skipped_count, rec.closure_count);
        assert_eq!(rec.swept_count, 0);
        assert_eq!(rec.cleanup_status, CleanupStatus::SkippedFastPath);
        assert_eq!(rec.cleanup_reason, CleanupReason::FastPath);
        assert_eq!(rec.error_stage, ErrorStage::None);
        assert_eq!(rec.authz_outcome, AuthzOutcome::Allow);
    }

    #[test]
    fn ok_non_fast_path_balances_accounting_and_defers_cleanup() {
        let rec = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42, 41]);
        rec.validate().expect("non-fast-path record is valid");
        assert_eq!(rec.linked_count + rec.skipped_count, rec.closure_count);
        assert!(!rec.fast_path);
        assert_eq!(rec.cleanup_status, CleanupStatus::DeferredAmbiguous);
        assert_eq!(
            rec.cleanup_reason,
            CleanupReason::RunningGenerationAmbiguous
        );
        assert_eq!(rec.sync_status, SyncStatus::Ok);
    }

    #[test]
    fn ok_cleanup_failed_keeps_activation_success() {
        let rec = StoreSyncAuditFields::ok_cleanup_failed(ctx(), 17, 0, vec![42, 41], 0, false);
        rec.validate()
            .expect("post-activation cleanup failure is valid");
        assert_eq!(rec.sync_status, SyncStatus::Ok);
        assert_eq!(rec.error_stage, ErrorStage::None);
        assert_eq!(rec.cleanup_status, CleanupStatus::Failed);
        assert_eq!(rec.cleanup_reason, CleanupReason::IoError);
    }

    #[test]
    fn failed_before_cleanup_uses_stage_and_no_cleanup() {
        for stage in [
            ErrorStage::Lock,
            ErrorStage::Probe,
            ErrorStage::Verify,
            ErrorStage::Stage,
            ErrorStage::Rename,
            ErrorStage::Metadata,
            ErrorStage::Integrity,
            ErrorStage::CurrentSwap,
            ErrorStage::Marker,
        ] {
            let rec = StoreSyncAuditFields::failed(ctx(), stage);
            rec.validate().expect("failed record is valid");
            assert_eq!(rec.sync_status, SyncStatus::Failed);
            assert_eq!(rec.error_stage, stage);
            assert_eq!(rec.cleanup_status, CleanupStatus::NotAttempted);
            assert_eq!(rec.cleanup_reason, CleanupReason::None);
            assert_eq!(rec.authz_outcome, AuthzOutcome::Allow);
        }
    }

    #[test]
    fn denied_is_authz_stage_with_deny_outcome() {
        let rec = StoreSyncAuditFields::denied(ctx());
        rec.validate().expect("denied record is valid");
        assert_eq!(rec.sync_status, SyncStatus::Failed);
        assert_eq!(rec.error_stage, ErrorStage::Authz);
        assert_eq!(rec.authz_outcome, AuthzOutcome::Deny);
        assert_eq!(rec.cleanup_status, CleanupStatus::NotAttempted);
        assert_eq!(rec.cleanup_reason, CleanupReason::None);
    }

    #[test]
    fn ok_with_non_none_error_stage_is_rejected() {
        let mut rec = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42]);
        rec.error_stage = ErrorStage::Stage;
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::OkWithErrorStage(ErrorStage::Stage))
        );
    }

    #[test]
    fn ok_accounting_mismatch_is_rejected() {
        let mut rec = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 5, vec![42]);
        // 5 + 5 != 17
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::OkAccountingMismatch {
                linked_count: 5,
                skipped_count: 5,
                closure_count: 17,
            })
        );
        // Cleanup-failed records are exempt from the accounting equality.
        rec.cleanup_status = CleanupStatus::Failed;
        rec.cleanup_reason = CleanupReason::IoError;
        rec.validate()
            .expect("cleanup-failed ok record is exempt from accounting equality");
    }

    #[test]
    fn fast_path_with_links_is_rejected() {
        let mut rec = StoreSyncAuditFields::ok_fast_path(ctx(), vec![42]);
        rec.linked_count = 3;
        rec.skipped_count = rec.closure_count - 3;
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::FastPathAccounting {
                linked_count: 3,
                skipped_count: 14,
                closure_count: 17,
            })
        );
    }

    #[test]
    fn skipped_fast_path_requires_no_sweep_and_fast_path_true() {
        let mut rec = StoreSyncAuditFields::ok_fast_path(ctx(), vec![42]);
        rec.swept_count = 2;
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::SkippedFastPathShape {
                fast_path: true,
                swept_count: 2,
            })
        );
    }

    #[test]
    fn every_valid_cleanup_pair_accepted_others_rejected() {
        use CleanupReason::*;
        use CleanupStatus::*;
        let valid = [
            (Completed, None),
            (NotAttempted, None),
            (DeferredOnline, VmRunning),
            (DeferredAmbiguous, RunningGenerationAmbiguous),
            (DeferredMetadata, MissingRetainedMetadata),
            (SkippedFastPath, FastPath),
            (Failed, IoError),
        ];
        for (s, r) in valid {
            assert!(
                cleanup_pair_is_valid(s, r),
                "expected ({s:?}, {r:?}) to be valid"
            );
        }
        // A representative cross-product of mismatched pairs is rejected.
        let invalid = [
            (Completed, IoError),
            (DeferredOnline, FastPath),
            (SkippedFastPath, None),
            (Failed, None),
            (DeferredAmbiguous, VmRunning),
            (NotAttempted, IoError),
        ];
        for (s, r) in invalid {
            assert!(
                !cleanup_pair_is_valid(s, r),
                "expected ({s:?}, {r:?}) to be rejected"
            );
        }
    }

    #[test]
    fn failed_shape_violation_is_rejected() {
        let mut rec = StoreSyncAuditFields::failed(ctx(), ErrorStage::Stage);
        // A failed record may not carry a deferred cleanup status.
        rec.cleanup_status = CleanupStatus::DeferredOnline;
        rec.cleanup_reason = CleanupReason::VmRunning;
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::FailedShape {
                error_stage: ErrorStage::Stage,
                cleanup_status: CleanupStatus::DeferredOnline,
                cleanup_reason: CleanupReason::VmRunning,
            })
        );
    }

    #[test]
    fn authz_stage_without_deny_is_rejected() {
        let mut rec = StoreSyncAuditFields::denied(ctx());
        rec.authz_outcome = AuthzOutcome::Allow;
        assert_eq!(
            rec.validate(),
            Err(StoreSyncAuditError::AuthzStageOutcomeMismatch {
                error_stage: ErrorStage::Authz,
                authz_outcome: AuthzOutcome::Allow,
            })
        );
    }

    #[test]
    fn enum_wire_strings_match_signed_schema() {
        assert_eq!(
            serde_json::to_value(SyncStatus::InProgress).unwrap(),
            serde_json::json!("in_progress")
        );
        assert_eq!(
            serde_json::to_value(ErrorStage::CurrentSwap).unwrap(),
            serde_json::json!("current_swap")
        );
        assert_eq!(
            serde_json::to_value(CleanupStatus::SkippedFastPath).unwrap(),
            serde_json::json!("skipped_fast_path")
        );
        assert_eq!(
            serde_json::to_value(CleanupReason::RunningGenerationAmbiguous).unwrap(),
            serde_json::json!("running_generation_ambiguous")
        );
        assert_eq!(
            serde_json::to_value(AuthzOutcome::Deny).unwrap(),
            serde_json::json!("deny")
        );
    }

    #[test]
    fn round_trips_through_json_with_deny_unknown_fields() {
        let rec = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42, 41]);
        let value = serde_json::to_value(&rec).unwrap();
        let back: StoreSyncAuditFields = serde_json::from_value(value).unwrap();
        assert_eq!(rec, back);
        // An extra field must be rejected (deny_unknown_fields).
        let mut obj = serde_json::to_value(&rec).unwrap();
        obj.as_object_mut()
            .unwrap()
            .insert("rogue".to_owned(), serde_json::json!(1));
        assert!(serde_json::from_value::<StoreSyncAuditFields>(obj).is_err());
    }
}
