//! Delivery tooling for the wave workflow described in
//! `docs/specs/ADR-046-validation-and-delivery.md`.
//!
//! The module is the shared skeleton every delivery subcommand hangs off:
//!
//! * [`model`] owns the digest and identifier contract (`content_id`,
//!   `candidate_id`, `snapshot_sha256`) from spec section 12.1.
//! * [`storage`] owns the external, candidate-ID-addressed evidence
//!   directory from spec sections 12.2 and 12.5. It is never under Git.
//! * [`command`] owns argument parsing, the `wave` subcommand table, and
//!   dispatch.
//! * [`snapshot`], [`evidence`], [`panel`], [`seal`], [`eligibility`], and
//!   [`history_proof`] carry one workflow stage each.
//!
//! Stages that are not implemented yet fail closed through
//! [`DeliveryError::unimplemented`]; no delivery subcommand ever reports
//! success without doing the work its name promises.

// Delivery contract symbols are published for the workflow stages that
// consume them; a stage that has not landed yet leaves its symbols unused.
#![allow(dead_code, unused_imports)]

use std::fmt;

pub mod command;
pub mod eligibility;
pub mod evidence;
pub mod history_proof;
pub mod model;
pub mod panel;
pub mod seal;
pub mod snapshot;
pub mod storage;

pub use command::{WaveCommand, WorkflowOutput, dispatch};
pub use model::{
    CandidateDigests, CandidateId, CandidateMaterial, ContentId, DependencyEdge, EvidenceResult,
    Fingerprint, GitObjectFormat, PANEL_MODEL_POLICY, PANEL_PROVIDER_POLICY,
    PANEL_REASONING_EFFORT_POLICY, PANEL_ROLES, PanelRole, RepositoryRecord, SnapshotSha256,
    canonical_digest,
};
pub use storage::{CandidateDir, StateRoot};

/// Schema version stamped into every delivery artifact and workflow result.
pub const DELIVERY_SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, DeliveryError>;

/// Failure classes the delivery CLI maps onto distinct exit codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryErrorKind {
    /// The invocation itself was malformed: unknown subcommand, missing or
    /// repeated option, unparsable value.
    Usage,
    /// The subcommand exists in the contract but its implementation has not
    /// landed yet. Always fails closed.
    Unimplemented,
    /// Input was well-formed but violated a delivery invariant.
    Invalid,
    /// The environment could not satisfy the request.
    Environment,
}

impl DeliveryErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Unimplemented => "unimplemented",
            Self::Invalid => "invalid",
            Self::Environment => "environment",
        }
    }

    /// Process exit code for this failure class.
    ///
    /// Each class maps to a distinct code drawn from the BSD `sysexits.h`
    /// range so a caller can branch on the reason without parsing stderr:
    ///
    /// | Class           | Code | `sysexits.h` name |
    /// | --------------- | ---- | ----------------- |
    /// | `Usage`         | `64` | `EX_USAGE`        |
    /// | `Invalid`       | `65` | `EX_DATAERR`      |
    /// | `Unimplemented` | `69` | `EX_UNAVAILABLE`  |
    /// | `Environment`   | `72` | `EX_OSFILE`       |
    ///
    /// The four codes are distinct and all nonzero, so success is never
    /// confused with a failure and no two classes share a code.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Usage => 64,
            Self::Invalid => 65,
            Self::Unimplemented => 69,
            Self::Environment => 72,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryError {
    kind: DeliveryErrorKind,
    message: String,
}

impl DeliveryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Invalid, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Usage, message)
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self::of(DeliveryErrorKind::Environment, message)
    }

    /// Fail-closed marker for a contract subcommand whose implementation has
    /// not landed yet.
    pub fn unimplemented(command: &str, work_item: &str) -> Self {
        Self::of(
            DeliveryErrorKind::Unimplemented,
            format!(
                "delivery wave {command} is not yet implemented \
                 (work item {work_item}); it fails closed rather than \
                 reporting an unearned success"
            ),
        )
    }

    pub fn of(kind: DeliveryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> DeliveryErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DeliveryError {}

impl From<std::io::Error> for DeliveryError {
    fn from(error: std::io::Error) -> Self {
        Self::of(
            DeliveryErrorKind::Environment,
            format!("I/O error: {error}"),
        )
    }
}

impl From<serde_json::Error> for DeliveryError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON error: {error}"))
    }
}

/// Entry point for `cargo xtask delivery <args...>`.
///
/// Renders the workflow result as one JSON object on stdout, or a diagnostic
/// on stderr with the failure class' nonzero exit code.
pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    match dispatch(args) {
        Ok(output) => match serde_json::to_string(&output) {
            Ok(json) => {
                println!("{json}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("delivery failed: cannot render result: {error}");
                std::process::ExitCode::from(DeliveryErrorKind::Invalid.exit_code())
            }
        },
        Err(error) => {
            eprintln!("delivery failed [{}]: {error}", error.kind().as_str());
            std::process::ExitCode::from(error.kind().exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_class_exits_nonzero() {
        for kind in [
            DeliveryErrorKind::Usage,
            DeliveryErrorKind::Unimplemented,
            DeliveryErrorKind::Invalid,
            DeliveryErrorKind::Environment,
        ] {
            assert_ne!(kind.exit_code(), 0, "{kind:?} must not exit zero");
        }
    }

    #[test]
    fn each_failure_class_maps_to_a_distinct_sysexits_code() {
        // The exact public contract: a caller branches on these codes, so both
        // the individual values and their mutual distinctness are load-bearing.
        assert_eq!(DeliveryErrorKind::Usage.exit_code(), 64);
        assert_eq!(DeliveryErrorKind::Invalid.exit_code(), 65);
        assert_eq!(DeliveryErrorKind::Unimplemented.exit_code(), 69);
        assert_eq!(DeliveryErrorKind::Environment.exit_code(), 72);

        let codes = [
            DeliveryErrorKind::Usage,
            DeliveryErrorKind::Unimplemented,
            DeliveryErrorKind::Invalid,
            DeliveryErrorKind::Environment,
        ]
        .map(DeliveryErrorKind::exit_code);
        let unique = codes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), codes.len(), "exit codes must be distinct");
        for code in codes {
            assert!(
                (64..=78).contains(&code),
                "{code} is outside the sysexits range"
            );
        }
    }

    #[test]
    fn unimplemented_error_names_its_command_and_work_item() {
        let error = DeliveryError::unimplemented("seal", "ADR046-delivery-006");
        assert_eq!(error.kind(), DeliveryErrorKind::Unimplemented);
        assert!(error.message().contains("seal"));
        assert!(error.message().contains("ADR046-delivery-006"));
    }
}
