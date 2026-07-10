use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use serde::{Deserialize, Serialize};

use crate::identity::ProxyIdentity;

pub const READINESS_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessStage {
    Upstream,
    Listener,
    FirstClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessFailure {
    UpstreamUnavailable,
    ListenerUnavailable,
    FirstClientTimeout,
    ClientRejected,
    ChannelUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessState {
    Ready,
    Failed,
}

/// Bounded, path-free readiness event consumed by the unsafe-local helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyReadinessEvent {
    pub protocol_version: u16,
    pub target: WorkloadTarget,
    pub provider_kind: WorkloadProviderKind,
    pub stage: ProxyReadinessStage,
    pub state: ProxyReadinessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ProxyReadinessFailure>,
}

impl ProxyReadinessEvent {
    pub fn ready(identity: &ProxyIdentity, stage: ProxyReadinessStage) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            target: identity.target().clone(),
            provider_kind: identity.provider_kind(),
            stage,
            state: ProxyReadinessState::Ready,
            failure: None,
        }
    }

    pub fn failed(
        identity: &ProxyIdentity,
        stage: ProxyReadinessStage,
        failure: ProxyReadinessFailure,
    ) -> Self {
        Self {
            protocol_version: READINESS_PROTOCOL_VERSION,
            target: identity.target().clone(),
            provider_kind: identity.provider_kind(),
            stage,
            state: ProxyReadinessState::Failed,
            failure: Some(failure),
        }
    }
}

#[derive(Debug)]
pub struct ReadinessReporter {
    identity: ProxyIdentity,
    stream: Option<UnixStream>,
}

impl ReadinessReporter {
    pub fn disabled(identity: ProxyIdentity) -> Self {
        Self {
            identity,
            stream: None,
        }
    }

    pub fn connect(identity: ProxyIdentity, path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(Duration::from_millis(250)))?;
        Ok(Self {
            identity,
            stream: Some(stream),
        })
    }

    #[cfg(test)]
    pub fn from_stream(identity: ProxyIdentity, stream: UnixStream) -> Self {
        Self {
            identity,
            stream: Some(stream),
        }
    }

    pub fn ready(&mut self, stage: ProxyReadinessStage) -> io::Result<()> {
        let event = ProxyReadinessEvent::ready(&self.identity, stage);
        self.emit(&event)
    }

    pub fn failed(
        &mut self,
        stage: ProxyReadinessStage,
        failure: ProxyReadinessFailure,
    ) -> io::Result<()> {
        let event = ProxyReadinessEvent::failed(&self.identity, stage, failure);
        self.emit(&event)
    }

    fn emit(&mut self, event: &ProxyReadinessEvent) -> io::Result<()> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        serde_json::to_writer(&mut *stream, event).map_err(io::Error::other)?;
        stream.write_all(b"\n")?;
        stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ProxyIdentity {
        ProxyIdentity::canonical(
            WorkloadTarget::parse("browser.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
        )
    }

    #[test]
    fn readiness_events_are_typed_and_do_not_carry_paths_or_argv() {
        let event = ProxyReadinessEvent::ready(&identity(), ProxyReadinessStage::Listener);
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""target":"browser.host.d2b""#));
        assert!(json.contains(r#""providerKind":"unsafe-local""#));
        assert!(json.contains(r#""stage":"listener""#));
        assert!(!json.contains("path"));
        assert!(!json.contains("argv"));
        assert!(!json.contains("command"));
        assert_eq!(
            serde_json::from_str::<ProxyReadinessEvent>(&json).unwrap(),
            event
        );
    }

    #[test]
    fn failed_readiness_has_only_closed_failure_reason() {
        let event = ProxyReadinessEvent::failed(
            &identity(),
            ProxyReadinessStage::FirstClient,
            ProxyReadinessFailure::FirstClientTimeout,
        );
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""failure":"first-client-timeout""#));
        assert!(!json.contains("/run/"));
    }
}
