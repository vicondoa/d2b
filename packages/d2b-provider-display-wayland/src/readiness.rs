//! Path-free proxy readiness events.

use serde::{Deserialize, Serialize};

use crate::process::{ProxyReadinessFailure, ProxyReadinessStage, ProxyReadinessState};

/// Version of the path-free readiness record.
pub const READINESS_PROTOCOL_VERSION: u16 = 1;

/// Readiness event emitted by the Host proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyReadinessEvent {
    /// Readiness protocol version.
    pub protocol_version: u16,
    /// Readiness stage.
    pub stage: ProxyReadinessStage,
    /// Readiness state.
    pub state: ProxyReadinessState,
    /// Closed failure reason, when failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ProxyReadinessFailure>,
}

impl ProxyReadinessEvent {
    /// Construct a Ready event.
    pub const fn ready(stage: ProxyReadinessStage) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            stage,
            state: ProxyReadinessState::Ready,
            failure: None,
        }
    }

    /// Construct a Failed event.
    pub const fn failed(stage: ProxyReadinessStage, failure: ProxyReadinessFailure) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            stage,
            state: ProxyReadinessState::Failed,
            failure: Some(failure),
        }
    }
}
