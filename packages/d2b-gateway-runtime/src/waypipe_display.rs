//! Waypipe display provider and runner shapes (ADR 0032, P0).
//!
//! P0 display forwarding is SHM-only: all runner command lines use
//! `waypipe --no-gpu`, never dmabuf/DRM/EGLStream, and clipboard is absent
//! unless a future explicit clipboard capability is added. This module keeps
//! command construction pure/tested; the daemon/broker wires the returned argv
//! into jailed `SpawnRunner` roles.

use async_trait::async_trait;
use d2b_constellation_core::{Capability, CapabilitySet, StreamDescriptor, StreamKind, StreamOpen};
use d2b_constellation_provider::capabilities::DisplayCapabilitySet;
use d2b_constellation_provider::error::{ProviderError, ProviderResult};
use d2b_constellation_provider::provider::DisplayProvider;
use d2b_constellation_provider::types::{
    DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest,
};

/// Waypipe compression selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaypipeCompression {
    /// zstd compression.
    Zstd,
    /// lz4 compression.
    Lz4,
    /// no compression.
    None,
}

impl WaypipeCompression {
    fn as_arg(self) -> &'static str {
        match self {
            WaypipeCompression::Zstd => "zstd",
            WaypipeCompression::Lz4 => "lz4",
            WaypipeCompression::None => "none",
        }
    }
}

/// Non-secret binary/path settings for Waypipe runners.
#[derive(Debug, Clone)]
pub struct WaypipeRunnerConfig {
    /// Waypipe binary path.
    pub waypipe_bin: String,
    /// Gateway display relay binary path.
    pub gateway_relay_bin: String,
    /// Host-side Unix socket for the operator compositor-side Waypipe client.
    pub host_socket: String,
    /// In-agent Wayland display name.
    pub display_name: String,
    /// Compression mode.
    pub compression: WaypipeCompression,
}

impl Default for WaypipeRunnerConfig {
    fn default() -> Self {
        Self {
            waypipe_bin: "waypipe".into(),
            gateway_relay_bin: "d2b-gateway-relay".into(),
            host_socket: "/run/d2b/gateway-display/waypipe.sock".into(),
            display_name: "wayland-d2b".into(),
            compression: WaypipeCompression::Zstd,
        }
    }
}

/// Host-side jailed Waypipe client runner (near the compositor).
pub fn host_waypipe_client_argv(cfg: &WaypipeRunnerConfig) -> Vec<String> {
    vec![
        cfg.waypipe_bin.clone(),
        "--no-gpu".into(),
        "-c".into(),
        cfg.compression.as_arg().into(),
        "-s".into(),
        cfg.host_socket.clone(),
        "client".into(),
    ]
}

/// Container/gateway-side Waypipe server runner. `socket` is the local socket
/// path that the gated relay sender exposes.
pub fn guest_waypipe_server_argv(cfg: &WaypipeRunnerConfig, socket: &str) -> Vec<String> {
    vec![
        cfg.waypipe_bin.clone(),
        "--no-gpu".into(),
        "-c".into(),
        cfg.compression.as_arg().into(),
        "-s".into(),
        socket.into(),
        "--display".into(),
        cfg.display_name.clone(),
        "server".into(),
        "--".into(),
        "sleep".into(),
        "infinity".into(),
    ]
}

/// The gated relay sender that bridges the Waypipe socket after the handshake
/// prologue verifies.
pub fn gated_relay_sender_argv(cfg: &WaypipeRunnerConfig) -> Vec<String> {
    vec![cfg.gateway_relay_bin.clone(), "sender".into()]
}

/// A systemd service unit shape for Waypipe. The ACA container cannot run
/// systemd, but the gateway guest and host side do; callers render these argv
/// into concrete unit files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaypipeSystemdService {
    /// Unit name.
    pub name: String,
    /// ExecStart argv.
    pub exec_start: Vec<String>,
    /// Restart policy.
    pub restart: String,
}

/// Host-side user/system service shape.
pub fn host_waypipe_service(cfg: &WaypipeRunnerConfig) -> WaypipeSystemdService {
    WaypipeSystemdService {
        name: "d2b-gateway-waypipe-client.service".into(),
        exec_start: host_waypipe_client_argv(cfg),
        restart: "on-failure".into(),
    }
}

/// Gateway-guest service shape.
pub fn guest_waypipe_service(cfg: &WaypipeRunnerConfig, socket: &str) -> WaypipeSystemdService {
    WaypipeSystemdService {
        name: "d2b-gateway-waypipe-server.service".into(),
        exec_start: guest_waypipe_server_argv(cfg, socket),
        restart: "on-failure".into(),
    }
}

/// Display provider backed by Waypipe over an already-authorized display
/// stream.
#[derive(Debug, Clone, Default)]
pub struct WaypipeDisplayProvider {
    cfg: WaypipeRunnerConfig,
}

impl WaypipeDisplayProvider {
    /// Build with explicit runner config.
    pub fn new(cfg: WaypipeRunnerConfig) -> Self {
        Self { cfg }
    }

    /// Runner config.
    pub fn config(&self) -> &WaypipeRunnerConfig {
        &self.cfg
    }
}

#[async_trait]
impl DisplayProvider for WaypipeDisplayProvider {
    fn provider_id(&self) -> d2b_constellation_core::ProviderId {
        d2b_constellation_core::ProviderId::parse("waypipe").expect("valid provider id")
    }

    fn capabilities(&self) -> DisplayCapabilitySet {
        DisplayCapabilitySet {
            caps: CapabilitySet::from_caps([Capability::WindowForwarding]),
            shm_buffers: true,
            dmabuf: false,
            reconnect: true,
        }
    }

    async fn open_display_session(
        &self,
        req: DisplaySessionRequest,
    ) -> ProviderResult<DisplaySessionHandle> {
        let expected = StreamOpen {
            descriptor: StreamDescriptor {
                id: req.display_stream.clone(),
                kind: StreamKind::Display,
            },
            operation_id: req.operation_id.clone(),
            authz: req.authz.clone(),
        };
        if !expected.is_consistent() {
            return Err(ProviderError::new(
                d2b_constellation_core::ErrorKind::Unauthorized,
                "display stream is not authorized for window forwarding",
            ));
        }
        Ok(DisplaySessionHandle {
            id: DisplaySessionId(format!("waypipe-{}", req.display_stream.as_str())),
        })
    }

    async fn close_display_session(&self, _id: DisplaySessionId) -> ProviderResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_constellation_core::{
        Capability, OperationId, PrincipalId, RealmPath, StreamAuthz, StreamId, WorkloadId,
    };

    #[test]
    fn argv_shapes_are_shm_only_no_gpu() {
        let cfg = WaypipeRunnerConfig::default();
        let host = host_waypipe_client_argv(&cfg);
        let guest = guest_waypipe_server_argv(&cfg, "/run/wp.sock");
        for argv in [&host, &guest] {
            assert!(argv.iter().any(|a| a == "--no-gpu"));
            assert!(!argv.iter().any(|a| a.contains("dmabuf")));
            assert!(!argv.iter().any(|a| a.contains("drm")));
            assert!(!argv.iter().any(|a| a.contains("clipboard")));
        }
        assert!(guest.windows(2).any(|w| w == ["--display", "wayland-d2b"]));
        assert_eq!(
            gated_relay_sender_argv(&cfg),
            ["d2b-gateway-relay", "sender"]
        );
    }

    #[test]
    fn systemd_service_shapes_restart_on_failure() {
        let cfg = WaypipeRunnerConfig::default();
        assert_eq!(host_waypipe_service(&cfg).restart, "on-failure");
        assert_eq!(
            guest_waypipe_service(&cfg, "/run/wp.sock").restart,
            "on-failure"
        );
    }

    #[tokio::test]
    async fn display_provider_requires_authorized_display_stream() {
        let provider = WaypipeDisplayProvider::default();
        let caps = provider.capabilities();
        assert!(caps.shm_buffers);
        assert!(!caps.dmabuf);
        assert!(caps.has(Capability::WindowForwarding));

        let stream = StreamId::parse("display-1").unwrap();
        let req = DisplaySessionRequest {
            workload: WorkloadId::parse("demo").unwrap(),
            operation_id: OperationId::parse("op-1").unwrap(),
            display_stream: stream.clone(),
            authz: StreamAuthz::for_kind(
                PrincipalId::parse("alice").unwrap(),
                RealmPath::local(),
                StreamKind::Display,
            ),
        };
        let handle = provider.open_display_session(req).await.unwrap();
        assert_eq!(handle.id.0, "waypipe-display-1");
    }

    #[tokio::test]
    async fn display_provider_rejects_inconsistent_stream_authz() {
        let provider = WaypipeDisplayProvider::default();
        let req = DisplaySessionRequest {
            workload: WorkloadId::parse("demo").unwrap(),
            operation_id: OperationId::parse("op-1").unwrap(),
            display_stream: StreamId::parse("display-1").unwrap(),
            authz: StreamAuthz {
                principal: PrincipalId::parse("alice").unwrap(),
                realm: RealmPath::local(),
                capability: Capability::Exec,
            },
        };
        let err = provider.open_display_session(req).await.unwrap_err();
        assert_eq!(err.kind(), d2b_constellation_core::ErrorKind::Unauthorized);
    }
}
