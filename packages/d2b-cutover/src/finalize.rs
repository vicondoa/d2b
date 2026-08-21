//! Separate phase-10 finalization contracts.

use std::fmt;

use crate::model::{CutoverPhase, FailureCode, OperationKind};

/// The only phase that may destroy legacy artifacts.
pub const FINALIZATION_PHASE: CutoverPhase = CutoverPhase::Finalization;

/// Validate that a finalization request is cutover-only and phase-10 bound.
pub fn require_cutover_finalization(
    operation_kind: OperationKind,
    phase: CutoverPhase,
) -> Result<(), FinalizationError> {
    if !operation_kind.is_cutover() {
        return Err(FinalizationError::ResetForbidden);
    }
    if phase != FINALIZATION_PHASE {
        return Err(FinalizationError::WrongPhase);
    }
    Ok(())
}

/// Finalization contract failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationError {
    /// Scoped resets cannot invoke cutover finalization.
    ResetForbidden,
    /// Finalization was requested outside phase 10.
    WrongPhase,
}

impl FinalizationError {
    /// Return the stable failure class.
    pub const fn code(self) -> FailureCode {
        match self {
            Self::ResetForbidden => FailureCode::EffectNotAllowed,
            Self::WrongPhase => FailureCode::InvalidTransition,
        }
    }
}

impl fmt::Display for FinalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResetForbidden => "reset cannot invoke cutover finalization",
            Self::WrongPhase => "finalization requires phase 10",
        })
    }
}

impl std::error::Error for FinalizationError {}
