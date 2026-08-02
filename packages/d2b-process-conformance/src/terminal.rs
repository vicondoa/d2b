//! Identity-bound terminal results for Process Providers.
//!
//! A readable pidfd is a liveness hint, not an exit-status proof.  The
//! `BrokerTerminalResult` type therefore accepts only evidence produced by
//! the parent that performed the wait/reap operation.  The evidence is
//! consumed when the result is relayed, so one result cannot be delivered
//! twice.

use std::fmt;

use d2b_contracts::v3::ResourceUid;

use crate::{
    ProcessConformanceError,
    identity::{ProcessIdentityDigest, WaitReapOwner},
    ticket::LaunchTicket,
};

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

    /// Construct a crash result without encoding a signal as an exit code.
    pub const fn crashed() -> Self {
        Self {
            exit_class: ExitClass::Crash,
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

    /// Construct an intentionally unclassified terminal result.
    pub const fn unknown() -> Self {
        Self {
            exit_class: ExitClass::Unknown,
            exit_code: None,
        }
    }

    /// Validate the relationship between terminal class and exit code.
    pub const fn validate(self) -> Result<(), ProcessConformanceError> {
        match (self.exit_class, self.exit_code) {
            (ExitClass::CleanExit, Some(code)) if code >= 0 && code <= 255 => Ok(()),
            (ExitClass::CleanExit, _) => Err(ProcessConformanceError::InvalidTerminalResult),
            (_, None) => Ok(()),
            (_, Some(_)) => Err(ProcessConformanceError::InvalidTerminalResult),
        }
    }
}

/// Evidence that the broker parent performed the terminal wait/reap.
///
/// The readable-pidfd form is intentionally not representable.  A test or
/// adapter may create this value only by passing a non-zero private token and
/// the exact identity it observed; it still cannot name a PID, path, or fd.
#[derive(PartialEq, Eq)]
pub struct ParentWaitEvidence {
    identity: ProcessIdentityDigest,
    operation_uid: ResourceUid,
    token: [u8; 32],
    reaped: bool,
    owner: WaitReapOwner,
}

impl ParentWaitEvidence {
    /// Record a verified parent wait/reap proof.
    pub fn verified(
        identity: ProcessIdentityDigest,
        operation_uid: ResourceUid,
        token: [u8; 32],
    ) -> Result<Self, ProcessConformanceError> {
        Self::verified_by(WaitReapOwner::Local, identity, operation_uid, token)
    }

    /// Record evidence from a named wait/reap owner.
    pub fn verified_by(
        owner: WaitReapOwner,
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
            owner,
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
            || evidence.owner != WaitReapOwner::Local
            || evidence.identity != identity
            || evidence.operation_uid != operation_uid
        {
            return Err(ProcessConformanceError::TerminalEvidenceMismatch);
        }
        outcome.validate()?;
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
            || ticket
                .expected_identity_digest()
                .is_some_and(|expected| expected != &self.identity)
        {
            return Err(ProcessConformanceError::TerminalEvidenceMismatch);
        }
        self.outcome.validate()?;
        Ok(self.outcome)
    }
}

impl fmt::Debug for BrokerTerminalResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrokerTerminalResult(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    fn identity(seed: u8) -> ProcessIdentityDigest {
        ProcessIdentityDigest::from_bytes([seed; 32])
    }

    #[test]
    fn terminal_results_require_parent_reap_and_matching_operation_evidence() {
        let ticket = fixtures::ticket_builder()
            .selected_provider("system-minijail")
            .expected_identity([
                crate::identity::IdentityBinding::Pid,
                crate::identity::IdentityBinding::ProcessStartTime,
            ])
            .build()
            .unwrap();
        let operation = ticket.operation().operation_uid().clone();
        let process = ticket.process_uid().clone();
        let evidence =
            ParentWaitEvidence::verified(identity(1), operation.clone(), [2; 32]).unwrap();
        let result = BrokerTerminalResult::from_parent(
            process.clone(),
            operation.clone(),
            identity(1),
            ProcessOutcome::exited(0).unwrap(),
            evidence,
        )
        .unwrap();
        assert_eq!(
            result.relay(&ticket).unwrap(),
            ProcessOutcome::exited(0).unwrap()
        );

        let wrong_owner = ParentWaitEvidence::verified_by(
            WaitReapOwner::ServiceManager,
            identity(1),
            operation.clone(),
            [2; 32],
        )
        .unwrap();
        assert_eq!(
            BrokerTerminalResult::from_parent(
                process,
                operation,
                identity(1),
                ProcessOutcome::exited(0).unwrap(),
                wrong_owner,
            )
            .unwrap_err(),
            ProcessConformanceError::TerminalEvidenceMismatch
        );
    }

    #[test]
    fn terminal_class_and_exit_code_are_bound_together() {
        assert!(ProcessOutcome::crashed().validate().is_ok());
        assert!(ProcessOutcome::unknown().validate().is_ok());
        assert_eq!(
            ProcessOutcome {
                exit_class: ExitClass::Crash,
                exit_code: Some(1),
            }
            .validate(),
            Err(ProcessConformanceError::InvalidTerminalResult)
        );
    }
}
