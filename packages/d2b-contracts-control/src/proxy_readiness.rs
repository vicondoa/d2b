//! Shared host-proxy readiness wire contract.

use d2b_contracts::{workload::WorkloadProviderKind, workload_identity::WorkloadTarget};
use serde::{Deserialize, Serialize};

/// Version of the path-free proxy readiness record.
pub const READINESS_PROTOCOL_VERSION: u16 = 1;

/// Proxy startup stage reported to the helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessStage {
    /// Upstream compositor connection is ready.
    Upstream,
    /// Guest-facing listener is ready.
    Listener,
    /// First guest client has connected.
    FirstClient,
}

/// Closed readiness state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessState {
    /// The stage completed successfully.
    Ready,
    /// The stage failed.
    Failed,
}

/// Closed failure reason for a readiness event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessFailure {
    /// Upstream compositor was unavailable.
    UpstreamUnavailable,
    /// Guest-facing listener could not be created.
    ListenerUnavailable,
    /// No first client arrived before the deadline.
    FirstClientTimeout,
    /// A guest client was rejected.
    ClientRejected,
    /// The readiness channel was unavailable.
    ChannelUnavailable,
}

/// Bounded, path-free readiness event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyReadinessEvent {
    /// Protocol version.
    pub protocol_version: u16,
    /// Canonical workload target.
    pub target: WorkloadTarget,
    /// Workload provider kind.
    pub provider_kind: WorkloadProviderKind,
    /// Startup stage.
    pub stage: ProxyReadinessStage,
    /// Startup state.
    pub state: ProxyReadinessState,
    /// Optional closed failure reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ProxyReadinessFailure>,
}

impl ProxyReadinessEvent {
    /// Construct a ready event for an authenticated workload identity.
    pub fn ready(
        target: WorkloadTarget,
        provider_kind: WorkloadProviderKind,
        stage: ProxyReadinessStage,
    ) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            target,
            provider_kind,
            stage,
            state: ProxyReadinessState::Ready,
            failure: None,
        }
    }

    /// Construct a failed event for an authenticated workload identity.
    pub fn failed(
        target: WorkloadTarget,
        provider_kind: WorkloadProviderKind,
        stage: ProxyReadinessStage,
        failure: ProxyReadinessFailure,
    ) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            target,
            provider_kind,
            stage,
            state: ProxyReadinessState::Failed,
            failure: Some(failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_round_trip_is_path_free() {
        let event = ProxyReadinessEvent::ready(
            WorkloadTarget::parse("browser.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
            ProxyReadinessStage::Listener,
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("/run/"));
        assert!(!json.contains("argv"));
        assert_eq!(
            serde_json::from_str::<ProxyReadinessEvent>(&json).unwrap(),
            event
        );
    }
}
