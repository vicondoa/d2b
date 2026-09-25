use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use wayland_proxy::identity::ProxyIdentity;
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

    /// Connect the readiness reporter to the daemon socket.
    ///
    /// The readiness reporter is a sync public surface; it is driven by the CLI
    /// binary's poll loop and has no async form at this boundary.
    ///
    /// # Errors
    ///
    /// Returns the underlying io error when the socket cannot be connected or
    /// its write timeout cannot be set.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn connect(identity: ProxyIdentity, path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(Duration::from_millis(250)))?;
        Ok(Self {
            identity,
            stream: Some(stream),
        })
    }

    /// Report a readiness stage transition.
    ///
    /// # Errors
    ///
    /// Returns the underlying io error when the event cannot be written to the
    /// connected socket.
    pub fn ready(&mut self, stage: ProxyReadinessStage) -> io::Result<()> {
        let event = ProxyReadinessEvent::ready(
            self.identity.target().clone(),
            self.identity.provider_kind(),
            stage,
        );
        self.emit(&event)
    }

    /// Report a readiness failure at a stage.
    ///
    /// # Errors
    ///
    /// Returns the underlying io error when the event cannot be written to the
    /// connected socket.
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

    // Short socket write against a 250 ms write timeout at the sync readiness
    // surface; no async form fits the CLI reporter path.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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
