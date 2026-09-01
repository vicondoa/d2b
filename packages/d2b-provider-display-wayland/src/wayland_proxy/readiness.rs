use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use crate::wayland_proxy::identity::ProxyIdentity;
pub use d2b_contracts_control::proxy_readiness::{
    ProxyReadinessEvent, ProxyReadinessFailure, ProxyReadinessStage,
};

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

    pub fn ready(&mut self, stage: ProxyReadinessStage) -> io::Result<()> {
        let event = ProxyReadinessEvent::ready(
            self.identity.target().clone(),
            self.identity.provider_kind(),
            stage,
        );
        self.emit(&event)
    }

    pub fn failed(
        &mut self,
        stage: ProxyReadinessStage,
        failure: ProxyReadinessFailure,
    ) -> io::Result<()> {
        let event = ProxyReadinessEvent::failed(
            self.identity.target().clone(),
            self.identity.provider_kind(),
            stage,
            failure,
        );
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
    use d2b_contracts::{workload::WorkloadProviderKind, workload_identity::WorkloadTarget};

    fn identity() -> ProxyIdentity {
        ProxyIdentity::canonical(
            WorkloadTarget::parse("browser.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
        )
    }

    #[test]
    fn readiness_events_are_typed_and_do_not_carry_paths_or_argv() {
        let identity = identity();
        let event = ProxyReadinessEvent::ready(
            identity.target().clone(),
            identity.provider_kind(),
            ProxyReadinessStage::Listener,
        );
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
        let identity = identity();
        let event = ProxyReadinessEvent::failed(
            identity.target().clone(),
            identity.provider_kind(),
            ProxyReadinessStage::FirstClient,
            ProxyReadinessFailure::FirstClientTimeout,
        );
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""failure":"first-client-timeout""#));
        assert!(!json.contains("/run/"));
    }
}
