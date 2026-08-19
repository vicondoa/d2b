#![deny(missing_docs)]

//! Pure cutover and scoped-reset contracts.

pub mod consent;
pub mod finalize;
pub mod hold;
pub mod inventory;
pub mod journal;
pub mod model;
pub mod preview;
pub mod reset;
pub mod rollback;
pub mod state_machine;
pub mod verify;

pub use consent::{
    Consent, ConsentBinding, ConsentError, FinalizationBinding, FinalizationConsent,
    RecoveryAttestation,
};
pub use finalize::{FINALIZATION_PHASE, FinalizationError};
pub use hold::{HoldError, HoldState};
pub use inventory::{
    HostInventory, InventoryClass, InventoryError, InventoryInputItem, InventoryItem, ZoneInventory,
};
pub use journal::{
    JOURNAL_DOMAIN, Journal, JournalBinding, JournalError, JournalRecord, JournalRecordKind,
};
pub use model::{
    ArtifactId, AuditEvidence, AuditRecordId, CandidateId, CompletionEvidence, CutoverPhase,
    Digest, Disposition, EffectEvidence, EffectId, EffectKind, EffectOutcome, FailureCode,
    HoldReason, HostLockContract, IdError, LockAcquire, LockId, OperationId, OperationKind,
    OperationState, OperatorId, RecoveryId, ReplayClass, ReplayDecision, ReplayObservation,
    ResetScope, RevisionPlanId, StepId, TerminalOutcomeKind, ZoneId,
};
pub use preview::{CutoverPreview, PREVIEW_DOMAIN, PreviewError, PreviewInventory};
pub use reset::{EffectAllowlist, EffectCapability, ResetError, ResetInventory, ResetTarget};
pub use rollback::{NATIVE_ROLLBACK_BOUNDARY, RollbackError, RollbackResult, plan_native_rollback};
pub use state_machine::{
    ApplyContext, CutoverEngine, EffectRequest, Operation, OperationError, OperationInventory,
    OperationRequest, ReadOnlyEvidence, ResetEngine,
};
pub use verify::{
    VerificationError, VerificationInput, VerificationReport, ZoneVerification, verify_cutover,
};
