//! Zone-wide EmergencyPolicy contract.

use d2b_contracts::wire_deserialize;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use d2b_contracts_resource::v3::execution_policy::redacted_debug;

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
        Self::new(false, EmergencyScope::default(), 30, "").expect("default is valid")
    }
}

wire_deserialize!(
    EmergencyPolicySpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        scope: EmergencyScope,
        #[serde(default = "default_deadline")]
        drain_deadline_seconds: u32,
        #[serde(default)]
        reason: String,
    },
    wire,
    Self::new(
        wire.enabled,
        wire.scope,
        wire.drain_deadline_seconds,
        wire.reason,
    )
    .map_err(serde::de::Error::custom)
);

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
    fn new_rejects_invalid_deadlines() {
        for deadline in [0, MAX_EMERGENCY_DRAIN_DEADLINE_SECONDS + 1, u32::MAX] {
            assert_eq!(
                EmergencyPolicySpec::new(false, EmergencyScope::default(), deadline, ""),
                Err(EmergencyPolicyContractError::InvalidDeadline),
                "deadline {deadline} must be rejected"
            );
        }
        assert!(
            EmergencyPolicySpec::new(
                false,
                EmergencyScope::default(),
                MAX_EMERGENCY_DRAIN_DEADLINE_SECONDS,
                ""
            )
            .is_ok()
        );
    }

    #[test]
    fn new_rejects_oversized_reason() {
        let reason = "x".repeat(MAX_EMERGENCY_REASON_BYTES + 1);
        assert_eq!(
            EmergencyPolicySpec::new(false, EmergencyScope::default(), 30, reason),
            Err(EmergencyPolicyContractError::ReasonTooLong)
        );
        assert!(
            EmergencyPolicySpec::new(
                false,
                EmergencyScope::default(),
                30,
                "x".repeat(MAX_EMERGENCY_REASON_BYTES)
            )
            .is_ok()
        );
    }

    #[test]
    fn new_rejects_control_characters_in_reason() {
        for character in ['\0', '\t', '\n', '\r', '\u{000b}', '\u{001b}', '\u{007f}'] {
            let reason = format!("drain{character}now");
            assert_eq!(
                EmergencyPolicySpec::new(false, EmergencyScope::default(), 30, reason),
                Err(EmergencyPolicyContractError::ReasonContainsControl),
                "control character {character:?} must be rejected"
            );
        }
    }

    #[test]
    fn new_accepts_plain_reason_and_retains_fields() {
        let spec =
            EmergencyPolicySpec::new(true, EmergencyScope::new(true, false, true, false), 45, "drain")
                .unwrap();
        assert!(spec.enabled());
        assert!(spec.scope().stop_new_admissions());
        assert!(!spec.scope().disconnect_zone_links());
        assert!(spec.scope().stop_provider_processes());
        assert_eq!(spec.drain_deadline_seconds(), 45);
        assert_eq!(spec.reason(), "drain");
    }

    #[test]
    fn effective_scope_is_none_without_enabled_policies() {
        assert_eq!(effective_scope([] as [&EmergencyPolicySpec; 0]), None);
        let disabled = EmergencyPolicySpec::new(false, EmergencyScope::new(true, false, false, false), 30, "")
            .unwrap();
        assert_eq!(effective_scope([&disabled]), None);
    }

    #[test]
    fn serde_default_round_trips_and_minimal_wire_deserializes() {
        let default = EmergencyPolicySpec::default();
        let wire = serde_json::to_string(&default).unwrap();
        assert_eq!(
            serde_json::from_str::<EmergencyPolicySpec>(&wire).unwrap(),
            default
        );
        assert_eq!(
            serde_json::from_str::<EmergencyPolicySpec>("{}").unwrap(),
            default
        );
    }

    #[test]
    fn serde_wire_applies_explicit_fields_and_rejects_invalid_deadline() {
        let spec = serde_json::from_str::<EmergencyPolicySpec>(
            "{\"enabled\":true,\"scope\":{\"stopNewAdmissions\":true},\"drainDeadlineSeconds\":45,\"reason\":\"drain\"}",
        )
        .unwrap();
        assert!(spec.enabled());
        assert!(spec.scope().stop_new_admissions());
        assert_eq!(spec.drain_deadline_seconds(), 45);
        assert_eq!(spec.reason(), "drain");
        assert!(serde_json::from_str::<EmergencyPolicySpec>(
            "{\"drainDeadlineSeconds\":0}"
        )
        .is_err());
    }
}
