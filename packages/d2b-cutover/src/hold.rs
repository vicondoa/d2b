//! Cutover-wide incident hold model.

use std::fmt;

use crate::model::{HoldReason, OperatorId};

/// The current hold state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldState {
    /// No hold is active.
    Clear,
    /// A hold is pending until the current atomic step completes.
    Pending {
        /// Operator who requested the hold.
        requested_by: OperatorId,
        /// Bounded operator reason.
        reason: HoldReason,
    },
    /// Destructive work is paused.
    Active {
        /// Operator who requested the hold.
        requested_by: OperatorId,
        /// Bounded operator reason.
        reason: HoldReason,
    },
}

impl HoldState {
    /// Return whether a hold blocks starting another effect.
    pub const fn blocks_new_effects(&self) -> bool {
        !matches!(self, Self::Clear)
    }

    /// Return whether a hold is active after an atomic step.
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    /// Return the operator who requested the hold.
    pub fn requested_by(&self) -> Option<&OperatorId> {
        match self {
            Self::Clear => None,
            Self::Pending { requested_by, .. } | Self::Active { requested_by, .. } => {
                Some(requested_by)
            }
        }
    }

    /// Return the bounded reason.
    pub fn reason(&self) -> Option<&HoldReason> {
        match self {
            Self::Clear => None,
            Self::Pending { reason, .. } | Self::Active { reason, .. } => Some(reason),
        }
    }
}

/// Hold transition failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldError {
    /// A new hold was requested after terminal completion.
    Terminal,
    /// Resume was requested by a different operator.
    OperatorMismatch,
    /// There was no active hold to resume.
    NotActive,
}

impl fmt::Display for HoldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Terminal => "hold is unavailable after terminal completion",
            Self::OperatorMismatch => "hold operator mismatch",
            Self::NotActive => "no active hold",
        })
    }
}

impl std::error::Error for HoldError {}
