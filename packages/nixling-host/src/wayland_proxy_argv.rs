//! `nixling-wayland-filter` host-side Wayland proxy argv generator.
//!
//! Pure Rust function that emits the argv for the per-VM
//! `nixling-<vm>-wlproxy` role spawned by `nixling-priv-broker`
//! via `SpawnRunner { role: WaylandProxy }`.
//!
//! The proxy runs as a dedicated `nixling-<vm>-wlproxy` UID with:
//!   - empty host capabilities (mandatory);
//!   - mandatory seccomp policy (w1-wayland-proxy);
//!   - no PipeWire/Pulse socket access;
//!   - dedicated per-VM runtime dir `/run/nixling-wlproxy/<vm>`;
//!   - host compositor socket bind-mounted at the in-jail upstream
//!     path `/run/nixling-wlproxy/<vm>/upstream`.
//!
//! Shape (Wave 2 / Lane A will fill the binary and policy):
//!
//! ```text
//! nixling-wayland-filter \
//!   --listen /run/nixling-wlproxy/<vm>/wayland-0 \
//!   --connect /run/nixling-wlproxy/<vm>/upstream \
//!   --vm-name <vm>
//! ```
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

// =========================================================================
// Input / output types
// =========================================================================

/// Input parameters for the wayland-proxy argv generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaylandProxyArgvInput {
    /// VM name (used to derive socket paths and the app-id prefix).
    pub vm_name: String,
    /// In-jail path for the filter listen socket (where crosvm connects).
    ///
    /// Default: `/run/nixling-wlproxy/<vm>/wayland-0`
    pub listen_socket: String,
    /// In-jail path for the upstream host compositor socket bind-mount.
    ///
    /// Default: `/run/nixling-wlproxy/<vm>/upstream`
    pub upstream_socket: String,
    /// App-id prefix injected by the proxy for all guest toplevels.
    ///
    /// Default: `nixling.<vm>.`
    pub app_id_prefix: String,
    /// Title prefix prepended to guest window titles.
    ///
    /// Default: `[<vm>] `
    pub title_prefix: String,
}

impl WaylandProxyArgvInput {
    /// Construct a default-shaped input for `vm_name`.
    pub fn for_vm(vm_name: impl Into<String>) -> Self {
        let vm_name = vm_name.into();
        let listen_socket = format!("/run/nixling-wlproxy/{vm_name}/wayland-0");
        let upstream_socket = format!("/run/nixling-wlproxy/{vm_name}/upstream");
        let app_id_prefix = format!("nixling.{vm_name}.");
        let title_prefix = format!("[{vm_name}] ");
        Self {
            vm_name,
            listen_socket,
            upstream_socket,
            app_id_prefix,
            title_prefix,
        }
    }
}

/// Errors from the wayland-proxy argv generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum WaylandProxyArgvError {
    /// The VM name is empty.
    EmptyVmName,
    /// A required socket path is not an absolute path.
    RelativeSocketPath { path: String },
}

/// Generate the argv for the wayland-proxy sidecar.
///
/// The returned vec's first element is `argv[0]` (the process title);
/// subsequent elements are the flag arguments consumed by
/// `nixling-wayland-filter`.
pub fn generate_wayland_proxy_argv(
    input: &WaylandProxyArgvInput,
) -> Result<Vec<String>, WaylandProxyArgvError> {
    if input.vm_name.is_empty() {
        return Err(WaylandProxyArgvError::EmptyVmName);
    }
    for path in [&input.listen_socket, &input.upstream_socket] {
        if !path.starts_with('/') {
            return Err(WaylandProxyArgvError::RelativeSocketPath { path: path.clone() });
        }
    }

    let mut argv = vec![
        format!("nixling-{}-wlproxy", input.vm_name),
        "--listen".to_owned(),
        input.listen_socket.clone(),
        "--connect".to_owned(),
        input.upstream_socket.clone(),
        "--vm-name".to_owned(),
        input.vm_name.clone(),
    ];

    if !input.app_id_prefix.is_empty() {
        argv.push("--app-id-prefix".to_owned());
        argv.push(input.app_id_prefix.clone());
    }
    if !input.title_prefix.is_empty() {
        argv.push("--title-prefix".to_owned());
        argv.push(input.title_prefix.clone());
    }

    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_argv_shape() {
        let input = WaylandProxyArgvInput::for_vm("work");
        let argv = generate_wayland_proxy_argv(&input).expect("valid input");
        assert_eq!(argv[0], "nixling-work-wlproxy", "argv[0] is process title");
        assert!(argv.contains(&"--listen".to_owned()));
        assert!(argv.contains(&"--connect".to_owned()));
        assert!(argv.contains(&"--vm-name".to_owned()));
        let listen_idx = argv.iter().position(|a| a == "--listen").unwrap();
        assert_eq!(argv[listen_idx + 1], "/run/nixling-wlproxy/work/wayland-0");
        let connect_idx = argv.iter().position(|a| a == "--connect").unwrap();
        assert_eq!(argv[connect_idx + 1], "/run/nixling-wlproxy/work/upstream");
    }

    #[test]
    fn empty_vm_name_errors() {
        let input = WaylandProxyArgvInput::for_vm("");
        assert_eq!(
            generate_wayland_proxy_argv(&input),
            Err(WaylandProxyArgvError::EmptyVmName)
        );
    }

    #[test]
    fn relative_socket_path_errors() {
        let mut input = WaylandProxyArgvInput::for_vm("work");
        input.listen_socket = "relative/path".to_owned();
        let err = generate_wayland_proxy_argv(&input).unwrap_err();
        assert!(matches!(
            err,
            WaylandProxyArgvError::RelativeSocketPath { .. }
        ));
    }

    #[test]
    fn app_id_prefix_in_argv() {
        let input = WaylandProxyArgvInput::for_vm("dev");
        let argv = generate_wayland_proxy_argv(&input).expect("valid");
        let prefix_idx = argv.iter().position(|a| a == "--app-id-prefix").unwrap();
        assert_eq!(argv[prefix_idx + 1], "nixling.dev.");
    }

    #[test]
    fn empty_app_id_prefix_omitted() {
        let mut input = WaylandProxyArgvInput::for_vm("work");
        input.app_id_prefix = String::new();
        let argv = generate_wayland_proxy_argv(&input).expect("valid");
        assert!(!argv.contains(&"--app-id-prefix".to_owned()));
    }

    #[test]
    fn title_prefix_in_argv() {
        let input = WaylandProxyArgvInput::for_vm("dev");
        let argv = generate_wayland_proxy_argv(&input).expect("valid");
        let prefix_idx = argv.iter().position(|a| a == "--title-prefix").unwrap();
        assert_eq!(argv[prefix_idx + 1], "[dev] ");
    }
}
