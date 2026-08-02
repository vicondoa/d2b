//! Identity-bound terminal results for Process Providers.
//!
//! A readable pidfd is a liveness hint, not an exit-status proof.  The
//! `BrokerTerminalResult` type therefore accepts only evidence produced by
//! the parent that performed the wait/reap operation.  The evidence is
//! consumed when the result is relayed, so one result cannot be delivered
//! twice.

use std::fmt;

use d2b_contracts::v3::ResourceUid;

use crate::{ProcessConformanceError, identity::ProcessIdentityDigest, ticket::LaunchTicket};

/// Stable terminal classification used by Process and EphemeralProcess
/// status projections.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExitClass {
    /// The process returned a normal exit status.
    CleanExit,
    /// The process ended with a crash or signal.
    Crash,
    /// The process was intentionally terminated by a signal.
    Signal,
    /// The provider stopped the process at its runtime deadline.
    Timeout,
    /// The parent could not classify the terminal state.
    Unknown,
}

/// A bounded terminal outcome detached from any process locator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutcome {
    /// The stable terminal classification.
    pub exit_class: ExitClass,
    /// The normal exit status, when one was collected by wait/reap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl ProcessOutcome {
    /// Construct a normal exit result.
    pub fn exited(exit_code: i32) -> Result<Self, ProcessConformanceError> {
        if !(0..=255).contains(&exit_code) {
            return Err(ProcessConformanceError::InvalidTerminalResult);
        }
        Ok(Self {
            exit_class: ExitClass::CleanExit,
            exit_code: Some(exit_code),
        })
    }

    /// Construct a signal result without encoding `128 + signal`.
    pub const fn signaled() -> Self {
        Self {
            exit_class: ExitClass::Signal,
            exit_code: None,
        }
    }

    /// Construct a timeout result.
    pub const fn timed_out() -> Self {
        Self {
            exit_class: ExitClass::Timeout,
            exit_code: None,
        }
    }
}

/// Evidence that the broker parent performed the terminal wait/reap.
///
/// The readable-pidfd form is intentionally not representable.  A test or
/// adapter may create this value only by passing a non-zero private token and
/// the exact identity it observed; it still cannot name a PID, path, or fd.
#[derive(Clone, PartialEq, Eq)]
pub struct ParentWaitEvidence {
    identity: ProcessIdentityDigest,
    operation_uid: ResourceUid,
    token: [u8; 32],
    reaped: bool,
}

impl ParentWaitEvidence {
    /// Record a verified parent wait/reap proof.
    pub fn verified(
        identity: ProcessIdentityDigest,
        operation_uid: ResourceUid,
        token: [u8; 32],
    ) -> Result<Self, ProcessConformanceError> {
        if identity.is_zero() || token == [0; 32] {
            return Err(ProcessConformanceError::InvalidTerminalResult);
        }
        Ok(Self {
            identity,
            operation_uid,
            token,
            reaped: true,
        })
    }

    /// Test whether this evidence proves a completed parent reap.
    pub const fn is_reaped(&self) -> bool {
        self.reaped
    }
}

impl fmt::Debug for ParentWaitEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ParentWaitEvidence(<redacted>)")
    }
}

/// The sole typed result a system-minijail broker parent may relay.
pub struct BrokerTerminalResult {
    process_uid: ResourceUid,
    operation_uid: ResourceUid,
    identity: ProcessIdentityDigest,
    outcome: ProcessOutcome,
    evidence: ParentWaitEvidence,
}

impl BrokerTerminalResult {
    /// Build a terminal result from parent-owned wait/reap evidence.
    pub fn from_parent(
        process_uid: ResourceUid,
        operation_uid: ResourceUid,
        identity: ProcessIdentityDigest,
        outcome: ProcessOutcome,
        evidence: ParentWaitEvidence,
    ) -> Result<Self, ProcessConformanceError> {
        if !evidence.is_reaped()
            || evidence.identity != identity
            || evidence.operation_uid != operation_uid
        {
            return Err(ProcessConformanceError::TerminalEvidenceMismatch);
        }
        Ok(Self {
            process_uid,
            operation_uid,
            identity,
            outcome,
            evidence,
        })
    }

    /// Borrow the process resource UID bound to this result.
    pub const fn process_uid(&self) -> &ResourceUid {
        &self.process_uid
    }

    /// Borrow the operation UID bound to this result.
    pub const fn operation_uid(&self) -> &ResourceUid {
        &self.operation_uid
    }

    /// Borrow the verified process identity.
    pub const fn identity(&self) -> &ProcessIdentityDigest {
        &self.identity
    }

    /// Borrow the collected terminal outcome.
    pub const fn outcome(&self) -> &ProcessOutcome {
        &self.outcome
    }

    /// Consume the result and relay it only to its matching launch ticket.
    pub fn relay(self, ticket: &LaunchTicket) -> Result<ProcessOutcome, ProcessConformanceError> {
        if !self.evidence.is_reaped()
            || ticket.process_uid() != &self.process_uid
            || ticket.operation().operation_uid() != &self.operation_uid
            || ticket.selected_provider().as_str() != "system-minijail"
        {
            return Err(ProcessConformanceError::TerminalEvidenceMismatch);
        }
        Ok(self.outcome)
    }
}

impl fmt::Debug for BrokerTerminalResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrokerTerminalResult(<redacted>)")
    }
}
