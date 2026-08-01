//! Fail-closed schema migration protocol for one Volume.
//!
//! Cross-Volume ordering is planned by the controller toolkit. This module
//! owns the volume-local side of staging, worker, commit, rollback, and restart
//! recovery. Every entry point requires a verified external identity marker.

use std::fmt;

use d2b_contracts::v3::{SchemaVersion, StateSchemaPhase};

use crate::audit::VolumeAuditKind;
use crate::marker::MarkerDisposition;

/// Signed migration worker template name.
pub const MIGRATION_WORKER_TEMPLATE: &str = "volume-migration-worker";
/// Source view mounted read-only by the migration worker.
pub const MIGRATION_SOURCE_VIEW: &str = "current";
/// Staging view mounted read-write by the migration worker.
pub const MIGRATION_STAGING_VIEW: &str = "staging";

/// Current durable phase of one Volume migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPhase {
    /// A schema difference was observed but no prepare condition is committed.
    Required,
    /// The prepare condition is committed and component mutation must stop.
    Preparing,
    /// The ephemeral staging Volume is being created.
    Staging,
    /// The signed migration worker is running.
    Migrating,
    /// Worker output is complete and awaits the atomic cutover.
    ReadyToCommit,
    /// A precommit failure is removing staging while active state is preserved.
    RollingBack,
    /// The target marker and active data are committed.
    Current,
    /// A precommit failure preserved the old schema and removed staging.
    Failed,
}

impl MigrationPhase {
    /// Project the protocol phase into the Volume state status contract.
    pub const fn state_schema_phase(self) -> StateSchemaPhase {
        match self {
            Self::Required => StateSchemaPhase::MigrationRequired,
            Self::Preparing
            | Self::Staging
            | Self::Migrating
            | Self::ReadyToCommit
            | Self::RollingBack => StateSchemaPhase::Migrating,
            Self::Current => StateSchemaPhase::Current,
            Self::Failed => StateSchemaPhase::MigrationFailed,
        }
    }
}

/// Next idempotent controller operation for one Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationAction {
    /// Commit the prepare condition before creating a staging Volume.
    CommitPrepare,
    /// Create the ephemeral staging Volume owned by the source Volume.
    CreateStaging,
    /// Dispatch the signed migration EphemeralProcess.
    DispatchWorker,
    /// Atomically replace active content with staged content.
    CommitStaging,
    /// Preserve active content and remove precommit staging content.
    RollbackStaging,
    /// Remove an orphaned staging Volume after a proven commit.
    CleanupStaging,
    /// No operation is required.
    None,
}

/// One migration transition and its lifecycle audit kind, when required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationTransition {
    /// New durable protocol phase.
    pub phase: MigrationPhase,
    /// Next idempotent controller operation.
    pub action: MigrationAction,
    /// Existing path-free lifecycle audit event kind.
    pub audit: Option<VolumeAuditKind>,
}

/// Closed, value-free migration failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationError {
    /// The source identity marker was not verified.
    MarkerNotVerified,
    /// The requested transition would downgrade installed state.
    DowngradeForbidden,
    /// The observed event is invalid for the current phase.
    InvalidTransition,
    /// Commit evidence did not prove the target marker version.
    TargetNotCommitted,
}

impl MigrationError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MarkerNotVerified => "volume-migration-marker-not-verified",
            Self::DowngradeForbidden => "volume-migration-downgrade-forbidden",
            Self::InvalidTransition => "volume-migration-transition-invalid",
            Self::TargetNotCommitted => "volume-migration-target-not-committed",
        }
    }
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for MigrationError {}

/// Stateful protocol for one source and staging Volume pair.
pub struct MigrationState {
    installed: SchemaVersion,
    target: SchemaVersion,
    phase: MigrationPhase,
}

impl MigrationState {
    /// Begin tracking a required forward migration after marker verification.
    pub fn new(
        installed: SchemaVersion,
        target: SchemaVersion,
        marker: MarkerDisposition,
    ) -> Result<Self, MigrationError> {
        require_verified(marker)?;
        if installed >= target {
            return Err(MigrationError::DowngradeForbidden);
        }
        Ok(Self {
            installed,
            target,
            phase: MigrationPhase::Required,
        })
    }

    /// Return the current protocol phase.
    pub const fn phase(&self) -> MigrationPhase {
        self.phase
    }

    /// Return the installed schema version that remains authoritative precommit.
    pub const fn installed_version(&self) -> SchemaVersion {
        self.installed
    }

    /// Return the desired forward target.
    pub const fn target_version(&self) -> SchemaVersion {
        self.target
    }

    /// Commit the coordinated prepare condition.
    pub fn prepare(&mut self) -> Result<MigrationTransition, MigrationError> {
        self.advance(
            MigrationPhase::Required,
            MigrationPhase::Preparing,
            MigrationAction::CommitPrepare,
            Some(VolumeAuditKind::VolumeMigrationStart),
        )
    }

    /// Record that all component writers acknowledged prepare.
    pub fn writers_ready(&mut self) -> Result<MigrationTransition, MigrationError> {
        self.advance(
            MigrationPhase::Preparing,
            MigrationPhase::Staging,
            MigrationAction::CreateStaging,
            None,
        )
    }

    /// Record that the staging Volume is ready and dispatch its worker.
    pub fn staging_ready(&mut self) -> Result<MigrationTransition, MigrationError> {
        self.advance(
            MigrationPhase::Staging,
            MigrationPhase::Migrating,
            MigrationAction::DispatchWorker,
            None,
        )
    }

    /// Record successful worker completion and request atomic cutover.
    pub fn worker_succeeded(&mut self) -> Result<MigrationTransition, MigrationError> {
        self.advance(
            MigrationPhase::Migrating,
            MigrationPhase::ReadyToCommit,
            MigrationAction::CommitStaging,
            None,
        )
    }

    /// Roll back a precommit worker failure while preserving active state.
    pub fn worker_failed(&mut self) -> Result<MigrationTransition, MigrationError> {
        if !matches!(
            self.phase,
            MigrationPhase::Preparing | MigrationPhase::Staging | MigrationPhase::Migrating
        ) {
            return Err(MigrationError::InvalidTransition);
        }
        self.phase = MigrationPhase::RollingBack;
        Ok(MigrationTransition {
            phase: self.phase,
            action: MigrationAction::RollbackStaging,
            audit: Some(VolumeAuditKind::VolumeMigrationFailed),
        })
    }

    /// Record durable staging cleanup after a precommit rollback.
    pub fn rollback_completed(&mut self) -> Result<MigrationTransition, MigrationError> {
        self.advance(
            MigrationPhase::RollingBack,
            MigrationPhase::Failed,
            MigrationAction::None,
            Some(VolumeAuditKind::VolumeMigrationRolledBack),
        )
    }

    /// Complete cutover only after the external marker proves the target.
    pub fn commit(
        &mut self,
        marker: MarkerDisposition,
        installed_after_commit: SchemaVersion,
    ) -> Result<MigrationTransition, MigrationError> {
        require_verified(marker)?;
        if self.phase != MigrationPhase::ReadyToCommit {
            return Err(MigrationError::InvalidTransition);
        }
        if installed_after_commit != self.target {
            return Err(MigrationError::TargetNotCommitted);
        }
        self.installed = installed_after_commit;
        self.phase = MigrationPhase::Current;
        Ok(MigrationTransition {
            phase: self.phase,
            action: MigrationAction::CleanupStaging,
            audit: Some(VolumeAuditKind::VolumeMigrationCommitted),
        })
    }

    fn advance(
        &mut self,
        expected: MigrationPhase,
        phase: MigrationPhase,
        action: MigrationAction,
        audit: Option<VolumeAuditKind>,
    ) -> Result<MigrationTransition, MigrationError> {
        if self.phase != expected {
            return Err(MigrationError::InvalidTransition);
        }
        self.phase = phase;
        Ok(MigrationTransition {
            phase,
            action,
            audit,
        })
    }
}

impl fmt::Debug for MigrationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationState")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Reconcile an interrupted commit from marker and staging observations.
pub fn recover_after_restart(
    marker: MarkerDisposition,
    installed: SchemaVersion,
    target: SchemaVersion,
    staging_exists: bool,
) -> Result<MigrationTransition, MigrationError> {
    require_verified(marker)?;
    if installed > target {
        return Err(MigrationError::DowngradeForbidden);
    }
    if installed == target {
        return Ok(MigrationTransition {
            phase: MigrationPhase::Current,
            action: if staging_exists {
                MigrationAction::CleanupStaging
            } else {
                MigrationAction::None
            },
            audit: None,
        });
    }
    Ok(MigrationTransition {
        phase: MigrationPhase::Migrating,
        action: if staging_exists {
            MigrationAction::DispatchWorker
        } else {
            MigrationAction::CreateStaging
        },
        audit: None,
    })
}

fn require_verified(marker: MarkerDisposition) -> Result<(), MigrationError> {
    if marker == MarkerDisposition::Verified {
        Ok(())
    } else {
        Err(MigrationError::MarkerNotVerified)
    }
}
