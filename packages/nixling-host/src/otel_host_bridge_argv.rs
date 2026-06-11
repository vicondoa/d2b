//! OTel host-bridge argv generator.
//!
//! Replaces the singleton `nixling-otel-host-bridge.service`
//! (`nixos-modules/components/observability/host.nix`) with a
//! broker-spawned runner under `RunnerRole::OtelHostBridge`.
//!
//! Per-role closed-set intent contract:
//!
//! - Pre-opened vsock fds only (no AF_VSOCK socket creation
//!   capability in the role profile).
//! - Broker rejects bundle intent whose source VM ≠ obs VM
//!   (`observability.vmName`).
//! - Per-role caps = empty.
//! - Bind set in the jail: nixling OTel runtime dir (RW), obs VM CH
//!   vsock socket (connect), host-egress.sock (RW listen target). No
//!   `/dev` bind mounts.
//!
//! Byte-parity oracle:
//! [`tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt`].
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// Inputs to the OTel host-bridge argv generator. The opaque references
/// are bundle-resolved by the broker; only typed scalars cross the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtelHostBridgeArgvInputs {
    /// Path to the socat binary (resolved from the bundle's relay
    /// package). MUST be an absolute path.
    pub socat_path: String,
    /// Path where the host OTel egress UDS will be listened on
    /// (`/run/nixling/otel/host-egress.sock`). MUST be absolute +
    /// non-empty.
    pub host_egress_socket: String,
    /// Path to the obs VM's base CH vsock UDS
    /// (`/var/lib/nixling/vms/<obs-vm>/vsock.sock`). MUST be
    /// absolute + non-empty.
    pub obs_vsock_host_socket: String,
    /// Vsock port the obs VM listens on for OTLP ingress (default
    /// 14317). MUST satisfy `1..=65535`.
    pub obs_otlp_port: u32,
    /// Path to the `nixling-ch-vsock-connect` helper that speaks the
    /// CH textual protocol on the base UDS. MUST be absolute.
    pub ch_vsock_connect_path: String,
}

/// Validation errors for [`generate_otel_host_bridge_argv`].
///
/// Carries enough context for the broker to surface a typed error
/// envelope without leaking sensitive bundle-resolved paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum OtelHostBridgeArgvError {
    /// A path-shaped input was empty.
    EmptyPath { field: &'static str },
    /// A path-shaped input was not absolute (did not start with `/`).
    NonAbsolutePath { field: &'static str, value: String },
    /// `obs_otlp_port` was outside the valid TCP/vsock port range
    /// `1..=65535`.
    PortOutOfRange { value: u32 },
    /// A path contained a NUL byte (would be rejected by execve
    /// later; refuse early at the argv generator).
    NulInPath { field: &'static str },
}

impl std::fmt::Display for OtelHostBridgeArgvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath { field } => write!(f, "otel-host-bridge: `{field}` is empty"),
            Self::NonAbsolutePath { field, value } => write!(
                f,
                "otel-host-bridge: `{field}` must be absolute, got `{value}`"
            ),
            Self::PortOutOfRange { value } => write!(
                f,
                "otel-host-bridge: `obsOtlpPort` {value} is outside 1..=65535"
            ),
            Self::NulInPath { field } => {
                write!(f, "otel-host-bridge: `{field}` contains a NUL byte")
            }
        }
    }
}

impl std::error::Error for OtelHostBridgeArgvError {}

fn require_abs_path(field: &'static str, value: &str) -> Result<(), OtelHostBridgeArgvError> {
    if value.is_empty() {
        return Err(OtelHostBridgeArgvError::EmptyPath { field });
    }
    if value.contains('\0') {
        return Err(OtelHostBridgeArgvError::NulInPath { field });
    }
    if !value.starts_with('/') {
        return Err(OtelHostBridgeArgvError::NonAbsolutePath {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// Generate the OTel host-bridge argv. Byte-parity oracle:
/// `tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt`.
///
/// The shape matches the singleton service's `ExecStart` line
/// (`nixos-modules/components/observability/host.nix` lines 302-360)
/// byte-for-byte modulo fd-inheritance differences: the broker
/// pre-opens the vsock fds and the per-role profile forbids
/// `AF_VSOCK`/`AF_UNIX` socket creation, so the helper relies on
/// inherited file descriptors only.
pub fn generate_otel_host_bridge_argv(
    inputs: &OtelHostBridgeArgvInputs,
) -> Result<Vec<String>, OtelHostBridgeArgvError> {
    require_abs_path("socatPath", &inputs.socat_path)?;
    require_abs_path("hostEgressSocket", &inputs.host_egress_socket)?;
    require_abs_path("obsVsockHostSocket", &inputs.obs_vsock_host_socket)?;
    require_abs_path("chVsockConnectPath", &inputs.ch_vsock_connect_path)?;
    if inputs.obs_otlp_port == 0 || inputs.obs_otlp_port > 65535 {
        return Err(OtelHostBridgeArgvError::PortOutOfRange {
            value: inputs.obs_otlp_port,
        });
    }

    Ok(vec![
        inputs.socat_path.clone(),
        "-d".to_owned(),
        "-d".to_owned(),
        format!(
            "UNIX-LISTEN:{},fork,reuseaddr,mode=0660",
            inputs.host_egress_socket
        ),
        format!(
            "EXEC:\"{} {} {}\"",
            inputs.ch_vsock_connect_path, inputs.obs_vsock_host_socket, inputs.obs_otlp_port
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn happy_inputs() -> OtelHostBridgeArgvInputs {
        OtelHostBridgeArgvInputs {
            socat_path: "/run/current-system/sw/bin/socat".to_owned(),
            host_egress_socket: "/run/nixling/otel/host-egress.sock".to_owned(),
            obs_vsock_host_socket: "/var/lib/nixling/vms/sys-obs/vsock.sock".to_owned(),
            obs_otlp_port: 14317,
            ch_vsock_connect_path: "/run/current-system/sw/bin/nixling-ch-vsock-connect".to_owned(),
        }
    }

    #[test]
    fn argv_shape_matches_singleton_service() {
        let argv = generate_otel_host_bridge_argv(&happy_inputs()).expect("happy path");
        // The argv shape must match the singleton service's ExecStart
        // line byte-for-byte after fd-passing differences
        // (i.e. fd inheritance handled by broker, not socat options).
        assert_eq!(
            argv,
            vec![
                "/run/current-system/sw/bin/socat",
                "-d",
                "-d",
                "UNIX-LISTEN:/run/nixling/otel/host-egress.sock,fork,reuseaddr,mode=0660",
                "EXEC:\"/run/current-system/sw/bin/nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs/vsock.sock 14317\"",
            ]
        );
    }

    #[test]
    fn snapshot_line_for_golden_parity() {
        // Consumed by tests/otel-host-bridge-argv-shape.sh to
        // compare against
        // tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt.
        let argv = generate_otel_host_bridge_argv(&happy_inputs()).expect("happy path");
        println!("SNAPSHOT: {}", argv.join(" "));
    }

    #[test]
    fn rejects_relative_socat_path() {
        let mut inputs = happy_inputs();
        inputs.socat_path = "socat".to_owned();
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::NonAbsolutePath { field, value }) => {
                assert_eq!(field, "socatPath");
                assert_eq!(value, "socat");
            }
            other => panic!("expected NonAbsolutePath, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_host_egress_socket() {
        let mut inputs = happy_inputs();
        inputs.host_egress_socket.clear();
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::EmptyPath { field }) => {
                assert_eq!(field, "hostEgressSocket")
            }
            other => panic!("expected EmptyPath, got {other:?}"),
        }
    }

    #[test]
    fn rejects_relative_obs_vsock_host_socket() {
        let mut inputs = happy_inputs();
        inputs.obs_vsock_host_socket = "vsock.sock".to_owned();
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::NonAbsolutePath { field, .. }) => {
                assert_eq!(field, "obsVsockHostSocket")
            }
            other => panic!("expected NonAbsolutePath, got {other:?}"),
        }
    }

    #[test]
    fn rejects_relative_ch_vsock_connect_path() {
        let mut inputs = happy_inputs();
        inputs.ch_vsock_connect_path = "bin/helper".to_owned();
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::NonAbsolutePath { field, .. }) => {
                assert_eq!(field, "chVsockConnectPath")
            }
            other => panic!("expected NonAbsolutePath, got {other:?}"),
        }
    }

    #[test]
    fn rejects_port_zero() {
        let mut inputs = happy_inputs();
        inputs.obs_otlp_port = 0;
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::PortOutOfRange { value }) => assert_eq!(value, 0),
            other => panic!("expected PortOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn rejects_port_above_max() {
        let mut inputs = happy_inputs();
        inputs.obs_otlp_port = 65536;
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::PortOutOfRange { value }) => assert_eq!(value, 65536),
            other => panic!("expected PortOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn rejects_nul_in_path() {
        let mut inputs = happy_inputs();
        inputs.host_egress_socket = "/run/alloy/host\0egress.sock".to_owned();
        match generate_otel_host_bridge_argv(&inputs) {
            Err(OtelHostBridgeArgvError::NulInPath { field }) => {
                assert_eq!(field, "hostEgressSocket")
            }
            other => panic!("expected NulInPath, got {other:?}"),
        }
    }
}
