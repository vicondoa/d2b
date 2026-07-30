//! Public Process status projection.
//!
//! No PID, pidfd, unit name, path, argv, environment, terminal byte, or raw
//! Provider diagnostic is public status. Identity travels only as the
//! opaque digest.

use serde::Serialize;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};

use crate::identity::{ProcessIdentityDigest, WaitReapOwner};
use crate::ticket::CompiledDigests;

/// Coarse lifecycle phase of the launched process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessPhaseClass {
    /// Launch requested, identity not yet established.
    Pending,
    /// Identity established and the process is running, not yet ready.
    Running,
    /// The readiness condition holds.
    Ready,
    /// The process reached a terminal state.
    Terminal,
    /// Identity is ambiguous and the process is quarantined.
    Unknown,
}

/// How the last observed run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExitClass {
    /// Normal termination with a zero status.
    Success,
    /// Normal termination with a nonzero status.
    Failure,
    /// The process was terminated by a signal.
    Signaled,
    /// The end of the run could not be classified.
    Unknown,
}

/// One terminal observation.
///
/// `exit_code` is the optional signed 32-bit terminal exit status frozen by
/// D108. A POSIX Provider reports 0 to 255 for a normal exit and never uses
/// the `128 + signal` convention; signal termination sets
/// [`ExitClass::Signaled`] and carries no exit code here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitObservation {
    /// The exit classification.
    pub class: ExitClass,
    /// The terminal exit status, present only for a real normal exit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl ExitObservation {
    /// Build a normal-exit observation from a terminal status.
    pub const fn from_exit_code(code: i32) -> Self {
        Self {
            class: if code == 0 {
                ExitClass::Success
            } else {
                ExitClass::Failure
            },
            exit_code: Some(code),
        }
    }

    /// Build a signal-termination observation, which carries no exit code.
    pub const fn signaled() -> Self {
        Self {
            class: ExitClass::Signaled,
            exit_code: None,
        }
    }
}

/// Whether the reported process was adopted or quarantined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdoptionCondition {
    /// The process was launched by this controller instance.
    NotApplicable,
    /// A running process was adopted after full identity verification.
    Adopted,
    /// Identity was ambiguous; the process is held, never killed or reused.
    Quarantined,
}

/// The Provider-written Process status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStatusReport {
    /// The Process Provider implementation that owns this process.
    pub provider: BoundedToken,
    /// The opaque stable process identity.
    pub identity: ProcessIdentityDigest,
    /// Who calls `wait` and reaps.
    pub wait_reap_owner: WaitReapOwner,
    /// The Host or Guest the process runs on.
    pub execution_ref: ResourceRef,
    /// The resolved execution domain.
    pub domain: ExecutionDomain,
    /// The exact user identity for a user-domain process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_ref: Option<ResourceRef>,
    /// The compiled configuration and resource revision digests.
    pub digests: CompiledDigests,
    /// The coarse lifecycle phase.
    pub phase: ProcessPhaseClass,
    /// The last terminal observation, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<ExitObservation>,
    /// Whether the process was adopted or quarantined.
    pub adoption: AdoptionCondition,
}

impl Serialize for CompiledDigests {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CompiledDigests", 7)?;
        state.serialize_field("sandbox", &self.sandbox)?;
        state.serialize_field("budget", &self.budget)?;
        state.serialize_field("mounts", &self.mounts)?;
        state.serialize_field("devices", &self.devices)?;
        state.serialize_field("network", &self.network)?;
        state.serialize_field("endpoints", &self.endpoints)?;
        state.serialize_field("fdTable", &self.fd_table)?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_observations_never_encode_a_signal_in_the_exit_code() {
        assert_eq!(
            ExitObservation::signaled(),
            ExitObservation {
                class: ExitClass::Signaled,
                exit_code: None,
            }
        );
        assert_eq!(ExitObservation::from_exit_code(0).class, ExitClass::Success);
        assert_eq!(ExitObservation::from_exit_code(3).class, ExitClass::Failure);
        let rendered = serde_json::to_string(&ExitObservation::signaled()).unwrap();
        assert!(!rendered.contains("exitCode"));
    }
}
