//! Zone-wide EmergencyPolicy contract.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{Timestamp, execution_policy::redacted_debug};

/// Canonical EmergencyPolicy ResourceType name.
pub const EMERGENCY_POLICY_RESOURCE_TYPE: &str = "EmergencyPolicy";
/// Core finalizer used while an active policy drains.
pub const EMERGENCY_DRAIN_FINALIZER: &str = "core.emergency-drain";
/// Maximum drain deadline in seconds.
pub const MAX_EMERGENCY_DRAIN_DEADLINE_SECONDS: u32 = 300;
/// Maximum reason bytes.
pub const MAX_EMERGENCY_REASON_BYTES: usize = 256;

/// Emergency scope flags.  The effective scope is the boolean union of all
/// enabled policies in a Zone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmergencyScope {
    #[serde(default)]
    stop_new_admissions: bool,
    #[serde(default)]
    disconnect_zone_links: bool,
    #[serde(default)]
    stop_provider_processes: bool,
    #[serde(default)]
    drain_ongoing_operations: bool,
}

impl EmergencyScope {
    /// Construct scope flags.
    pub const fn new(
        stop_new_admissions: bool,
        disconnect_zone_links: bool,
        stop_provider_processes: bool,
        drain_ongoing_operations: bool,
    ) -> Self {
        Self {
            stop_new_admissions,
            disconnect_zone_links,
            stop_provider_processes,
            drain_ongoing_operations,
        }
    }

    /// Union two scopes, retaining the most restrictive action.
    pub const fn union(self, other: Self) -> Self {
        Self {
            stop_new_admissions: self.stop_new_admissions || other.stop_new_admissions,
            disconnect_zone_links: self.disconnect_zone_links || other.disconnect_zone_links,
            stop_provider_processes: self.stop_provider_processes || other.stop_provider_processes,
            drain_ongoing_operations: self.drain_ongoing_operations
                || other.drain_ongoing_operations,
        }
    }

    /// Whether new admissions are stopped.
    pub const fn stop_new_admissions(self) -> bool {
        self.stop_new_admissions
    }

    /// Whether ZoneLinks are disconnected.
    pub const fn disconnect_zone_links(self) -> bool {
        self.disconnect_zone_links
    }

    /// Whether Provider component processes are stopped.
    pub const fn stop_provider_processes(self) -> bool {
        self.stop_provider_processes
    }

    /// Whether ongoing operations drain.
    pub const fn drain_ongoing_operations(self) -> bool {
        self.drain_ongoing_operations
    }
}

/// EmergencyPolicy schema failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyPolicyContractError {
    InvalidDeadline,
    ReasonTooLong,
    ReasonContainsControl,
}

impl core::fmt::Display for EmergencyPolicyContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDeadline => "emergency-drain-deadline-invalid",
            Self::ReasonTooLong => "emergency-reason-too-long",
            Self::ReasonContainsControl => "emergency-reason-invalid",
        })
    }
}

impl std::error::Error for EmergencyPolicyContractError {}

/// Complete EmergencyPolicy desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmergencyPolicySpec {
    enabled: bool,
    scope: EmergencyScope,
    drain_deadline_seconds: u32,
    reason: String,
}

impl EmergencyPolicySpec {
    /// Construct and validate an emergency policy.
    pub fn new(
        enabled: bool,
        scope: EmergencyScope,
        drain_deadline_seconds: u32,
        reason: impl Into<String>,
    ) -> Result<Self, EmergencyPolicyContractError> {
        let reason = reason.into();
        if drain_deadline_seconds == 0
            || drain_deadline_seconds > MAX_EMERGENCY_DRAIN_DEADLINE_SECONDS
        {
            return Err(EmergencyPolicyContractError::InvalidDeadline);
        }
        if reason.len() > MAX_EMERGENCY_REASON_BYTES {
            return Err(EmergencyPolicyContractError::ReasonTooLong);
        }
        if reason
            .chars()
            .any(|character| character.is_control() || character == '\u{007f}')
        {
            return Err(EmergencyPolicyContractError::ReasonContainsControl);
        }
        Ok(Self {
            enabled,
            scope,
            drain_deadline_seconds,
            reason,
        })
    }

    /// Construct the inactive default policy.
    pub fn default_values() -> Self {
        Self::new(false, EmergencyScope::default(), 30, "").expect("default is valid")
    }

    /// Whether this policy contributes to the effective scope.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Return scope flags.
    pub const fn scope(&self) -> EmergencyScope {
        self.scope
    }

    /// Return drain deadline in seconds.
    pub const fn drain_deadline_seconds(&self) -> u32 {
        self.drain_deadline_seconds
    }

    /// Borrow the operator rationale.  This value is deliberately not
    /// included by any status or metric adapter.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

redacted_debug!(EmergencyPolicySpec);

impl Default for EmergencyPolicySpec {
    fn default() -> Self {
        Self::default_values()
    }
}

impl<'de> Deserialize<'de> for EmergencyPolicySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            enabled: bool,
            #[serde(default)]
            scope: EmergencyScope,
            #[serde(default = "default_deadline")]
            drain_deadline_seconds: u32,
            #[serde(default)]
            reason: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.enabled,
            wire.scope,
            wire.drain_deadline_seconds,
            wire.reason,
        )
        .map_err(serde::de::Error::custom)
    }
}

const fn default_deadline() -> u32 {
    30
}

/// Closed EmergencyPolicy condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EmergencyPolicyConditionType {
    PolicyValid,
    Enforced,
    DrainComplete,
    EmergencyDrainPending,
}

/// ResourceType-common EmergencyPolicy status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmergencyPolicyStatusResource {
    active: bool,
    activated_at: Option<Timestamp>,
    deactivated_at: Option<Timestamp>,
    drain_completed_at: Option<Timestamp>,
}

impl EmergencyPolicyStatusResource {
    /// Construct the identity-free status projection.
    pub const fn new(
        active: bool,
        activated_at: Option<Timestamp>,
        deactivated_at: Option<Timestamp>,
        drain_completed_at: Option<Timestamp>,
    ) -> Self {
        Self {
            active,
            activated_at,
            deactivated_at,
            drain_completed_at,
        }
    }

    /// Whether effects are currently applied.
    pub const fn active(&self) -> bool {
        self.active
    }

    /// Borrow the most recent activation time.
    pub const fn activated_at(&self) -> Option<&Timestamp> {
        self.activated_at.as_ref()
    }

    /// Borrow the most recent deactivation time.
    pub const fn deactivated_at(&self) -> Option<&Timestamp> {
        self.deactivated_at.as_ref()
    }

    /// Borrow the drain completion time.
    pub const fn drain_completed_at(&self) -> Option<&Timestamp> {
        self.drain_completed_at.as_ref()
    }
}

/// Alias used by generic status adapters.
pub type EmergencyPolicyStatus = EmergencyPolicyStatusResource;

/// Compute the effective union and tightest deadline of enabled policies.
pub fn effective_scope<'a>(
    policies: impl IntoIterator<Item = &'a EmergencyPolicySpec>,
) -> Option<(EmergencyScope, u32)> {
    let mut effective: Option<EmergencyScope> = None;
    let mut deadline = u32::MAX;
    for policy in policies {
        if policy.enabled() {
            effective = Some(effective.unwrap_or_default().union(policy.scope()));
            deadline = deadline.min(policy.drain_deadline_seconds());
        }
    }
    effective.map(|scope| (scope, deadline))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_policy_union_is_most_restrictive_and_deadline_is_tightest() {
        let first =
            EmergencyPolicySpec::new(true, EmergencyScope::new(true, false, false, true), 30, "")
                .unwrap();
        let second =
            EmergencyPolicySpec::new(true, EmergencyScope::new(false, true, true, false), 5, "")
                .unwrap();
        let (scope, deadline) = effective_scope([&first, &second]).unwrap();
        assert!(scope.stop_new_admissions());
        assert!(scope.disconnect_zone_links());
        assert!(scope.stop_provider_processes());
        assert_eq!(deadline, 5);
    }

    #[test]
    fn reason_is_bounded_and_status_has_no_reason_field() {
        let spec = EmergencyPolicySpec::default();
        let encoded = serde_json::to_vec(&spec).unwrap();
        assert!(String::from_utf8(encoded).unwrap().contains("reason"));
        assert!(
            serde_json::to_string(&EmergencyPolicyStatusResource::new(false, None, None, None))
                .unwrap()
                .find("reason")
                .is_none()
        );
    }
}
