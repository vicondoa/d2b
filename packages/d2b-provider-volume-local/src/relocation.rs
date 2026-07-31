//! Marker-gated source Volume relocation protocol.
//!
//! Copying is performed by a signed EphemeralProcess over adapter-routed
//! anchored views. The source marker, bytes, and finalizer remain authoritative
//! until destination activation and attachment re-point both succeed.

use std::fmt;

use crate::audit::VolumeAuditKind;
use crate::marker::MarkerDisposition;

/// Signed relocation worker template name.
pub const RELOCATION_WORKER_TEMPLATE: &str = "volume-relocation-worker";
/// Source view mounted read-only by the relocation worker.
pub const RELOCATION_SOURCE_VIEW: &str = "relocation-source";
/// Destination view mounted read-write by the relocation worker.
pub const RELOCATION_DESTINATION_VIEW: &str = "relocation-dest";

/// Durable relocation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationPhase {
    /// No source finalizer is committed yet.
    Idle,
    /// The source finalizer must commit before process drain.
    FinalizerPending,
    /// Source mounts are being drained.
    Draining,
    /// Destination Volume provisioning is pending.
    DestinationPending,
    /// The signed anchored-copy worker is pending or running.
    Copying,
    /// Copy succeeded and destination activation is pending.
    DestinationActivationPending,
    /// Guest attachment exports must be re-pointed to the destination.
    AttachmentRepointPending,
    /// Destination and attachments are ready for source deletion.
    ReadyToCommit,
    /// Finalizer removal committed and source deletion may complete.
    SourceDeletionPending,
    /// Source deletion completed.
    Committed,
    /// Copy or activation failed; source and finalizer remain preserved.
    Failed,
}

/// Next idempotent relocation operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationAction {
    /// Commit the source `Relocating` finalizer.
    AddSourceFinalizer,
    /// Stop every Process mounting the source Volume.
    DrainSourceMounts,
    /// Create and marker-bind the destination Volume.
    CreateDestination,
    /// Dispatch the signed anchored-copy worker.
    DispatchCopyWorker,
    /// Activate the complete destination Volume.
    ActivateDestination,
    /// Reconcile virtiofs Export children against the destination source.
    RepointAttachments,
    /// Remove the source finalizer only after complete cutover.
    RemoveSourceFinalizer,
    /// Delete the unfinalized source Volume.
    DeleteSource,
    /// No further operation is required.
    None,
    /// Preserve source bytes, marker, and finalizer for operator recovery.
    PreserveSource,
}

/// One relocation transition and existing lifecycle audit kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocationTransition {
    /// New durable protocol phase.
    pub phase: RelocationPhase,
    /// Next controller operation.
    pub action: RelocationAction,
    /// Existing path-free lifecycle audit event kind.
    pub audit: Option<VolumeAuditKind>,
}

/// Closed relocation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationError {
    /// The source identity marker was not verified.
    MarkerNotVerified,
    /// An event was applied in the wrong phase.
    InvalidTransition,
}

impl RelocationError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MarkerNotVerified => "volume-relocation-marker-not-verified",
            Self::InvalidTransition => "volume-relocation-transition-invalid",
        }
    }
}

impl fmt::Display for RelocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for RelocationError {}

/// Relocation state machine for one source and destination pair.
pub struct RelocationState {
    phase: RelocationPhase,
    has_guest_attachments: bool,
}

impl RelocationState {
    /// Construct relocation state only for a verified source root.
    pub fn new(
        source_marker: MarkerDisposition,
        has_guest_attachments: bool,
    ) -> Result<Self, RelocationError> {
        require_verified(source_marker)?;
        Ok(Self {
            phase: RelocationPhase::Idle,
            has_guest_attachments,
        })
    }

    /// Return the current durable phase.
    pub const fn phase(&self) -> RelocationPhase {
        self.phase
    }

    /// Begin relocation by committing the source finalizer.
    pub fn begin(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::Idle,
            RelocationPhase::FinalizerPending,
            RelocationAction::AddSourceFinalizer,
            Some(VolumeAuditKind::VolumeRelocationStart),
        )
    }

    /// Record finalizer commit and request source drain.
    pub fn finalizer_committed(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::FinalizerPending,
            RelocationPhase::Draining,
            RelocationAction::DrainSourceMounts,
            None,
        )
    }

    /// Record complete drain and request destination creation.
    pub fn source_drained(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::Draining,
            RelocationPhase::DestinationPending,
            RelocationAction::CreateDestination,
            None,
        )
    }

    /// Dispatch the anchored-copy worker after destination marker verification.
    pub fn destination_ready(
        &mut self,
        destination_marker: MarkerDisposition,
    ) -> Result<RelocationTransition, RelocationError> {
        require_verified(destination_marker)?;
        self.advance(
            RelocationPhase::DestinationPending,
            RelocationPhase::Copying,
            RelocationAction::DispatchCopyWorker,
            None,
        )
    }

    /// Preserve the source after a worker or destination failure.
    pub fn copy_failed(&mut self) -> Result<RelocationTransition, RelocationError> {
        if !matches!(
            self.phase,
            RelocationPhase::Copying | RelocationPhase::DestinationActivationPending
        ) {
            return Err(RelocationError::InvalidTransition);
        }
        self.phase = RelocationPhase::Failed;
        Ok(RelocationTransition {
            phase: self.phase,
            action: RelocationAction::PreserveSource,
            audit: None,
        })
    }

    /// Record a complete copy and request destination activation.
    pub fn copy_succeeded(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::Copying,
            RelocationPhase::DestinationActivationPending,
            RelocationAction::ActivateDestination,
            None,
        )
    }

    /// Record destination activation before changing any source attachment.
    pub fn destination_activated(&mut self) -> Result<RelocationTransition, RelocationError> {
        if self.phase != RelocationPhase::DestinationActivationPending {
            return Err(RelocationError::InvalidTransition);
        }
        let (phase, action) = if self.has_guest_attachments {
            (
                RelocationPhase::AttachmentRepointPending,
                RelocationAction::RepointAttachments,
            )
        } else {
            (
                RelocationPhase::ReadyToCommit,
                RelocationAction::RemoveSourceFinalizer,
            )
        };
        self.phase = phase;
        Ok(RelocationTransition {
            phase,
            action,
            audit: None,
        })
    }

    /// Record that every Export child now targets the destination source.
    pub fn attachments_repointed(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::AttachmentRepointPending,
            RelocationPhase::ReadyToCommit,
            RelocationAction::RemoveSourceFinalizer,
            None,
        )
    }

    /// Request source deletion after finalizer removal commits.
    pub fn finalizer_removed(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::ReadyToCommit,
            RelocationPhase::SourceDeletionPending,
            RelocationAction::DeleteSource,
            None,
        )
    }

    /// Complete source deletion after finalizer removal.
    pub fn source_deleted(&mut self) -> Result<RelocationTransition, RelocationError> {
        self.advance(
            RelocationPhase::SourceDeletionPending,
            RelocationPhase::Committed,
            RelocationAction::None,
            Some(VolumeAuditKind::VolumeRelocationCommitted),
        )
    }

    fn advance(
        &mut self,
        expected: RelocationPhase,
        phase: RelocationPhase,
        action: RelocationAction,
        audit: Option<VolumeAuditKind>,
    ) -> Result<RelocationTransition, RelocationError> {
        if self.phase != expected {
            return Err(RelocationError::InvalidTransition);
        }
        self.phase = phase;
        Ok(RelocationTransition {
            phase,
            action,
            audit,
        })
    }
}

impl fmt::Debug for RelocationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelocationState")
            .field("phase", &self.phase)
            .field("has_guest_attachments", &self.has_guest_attachments)
            .finish()
    }
}

fn require_verified(marker: MarkerDisposition) -> Result<(), RelocationError> {
    if marker == MarkerDisposition::Verified {
        Ok(())
    } else {
        Err(RelocationError::MarkerNotVerified)
    }
}
