//! `d2b-wayland-proxy` host-side Wayland proxy argv generator.
//!
//! Pure Rust function that emits the argv for the per-VM
//! `d2b-<vm>-wlproxy` role spawned by `d2b-priv-broker`
//! via `SpawnRunner { role: WaylandProxy }`.
//!
//! The proxy runs as a dedicated `d2b-<vm>-wlproxy` UID with:
//!   - empty host capabilities (mandatory);
//!   - mandatory seccomp policy (w1-wayland-proxy);
//!   - no PipeWire/Pulse socket access;
//!   - dedicated per-VM runtime dir `/run/d2b-wlproxy/<vm>`;
//!   - host compositor socket bind-mounted at the in-jail upstream
//!     path `/run/d2b-wlproxy/<vm>/upstream`.
//!
//! Shape (Wave 2 / Lane A will fill the binary and policy):
//!
//! ```text
//! d2b-wayland-proxy \
//!   --listen /run/d2b-wlproxy/<vm>/wayland-0 \
//!   --connect /run/d2b-wlproxy/<vm>/upstream \
//!   --vm-name <vm>
//! ```
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use std::num::NonZeroU32;

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
    /// Default: `/run/d2b-wlproxy/<vm>/wayland-0`
    pub listen_socket: String,
    /// In-jail path for the upstream host compositor socket bind-mount.
    ///
    /// Default: `/run/d2b-wlproxy/<vm>/upstream`
    pub upstream_socket: String,
    /// App-id prefix injected by the proxy for all guest toplevels.
    ///
    /// Default: `d2b.<vm>.`
    pub app_id_prefix: String,
    /// Title prefix prepended to guest window titles.
    ///
    /// Default: `[<vm>] `
    pub title_prefix: String,
    /// Effective VM identity border configuration.
    ///
    /// Default: `None` (no border flags emitted). Nix enables borders by
    /// passing a populated config when the wayland-proxy node is emitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<WaylandProxyBorderConfig>,
}

/// Effective proxy-drawn VM identity border settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaylandProxyBorderConfig {
    /// Active/focused border color as `#rrggbb`.
    pub active_color: String,
    /// Inactive/unfocused border color as `#rrggbb`.
    pub inactive_color: String,
    /// Urgent border color as `#rrggbb`.
    pub urgent_color: String,
    /// Side/bottom border thickness in logical pixels.
    pub thickness: NonZeroU32,
    /// Label configuration. Defaults to enabled VM-name label.
    #[serde(default)]
    pub label: WaylandProxyBorderLabelConfig,
}

/// Proxy-drawn VM identity label settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaylandProxyBorderLabelConfig {
    /// Whether to emit label argv.
    pub enable: bool,
    /// Optional label override. `None` means the authenticated VM name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Label position.
    pub position: WaylandProxyBorderLabelPosition,
}

impl Default for WaylandProxyBorderLabelConfig {
    fn default() -> Self {
        Self {
            enable: true,
            text: None,
            position: WaylandProxyBorderLabelPosition::TopLeft,
        }
    }
}

/// Proxy-drawn VM identity label position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaylandProxyBorderLabelPosition {
    TopLeft,
    TopCenter,
}

impl WaylandProxyBorderLabelPosition {
    fn as_str(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
        }
    }
}

impl WaylandProxyArgvInput {
    /// Construct a default-shaped input for `vm_name`.
    pub fn for_vm(vm_name: impl Into<String>) -> Self {
        let vm_name = vm_name.into();
        let listen_socket = format!("/run/d2b-wlproxy/{vm_name}/wayland-0");
        let upstream_socket = format!("/run/d2b-wlproxy/{vm_name}/upstream");
        let app_id_prefix = format!("d2b.{vm_name}.");
        let title_prefix = format!("[{vm_name}] ");
        Self {
            vm_name,
            listen_socket,
            upstream_socket,
            app_id_prefix,
            title_prefix,
            border: None,
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
/// `d2b-wayland-proxy`.
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
        format!("d2b-{}-wlproxy", input.vm_name),
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
    if let Some(border) = &input.border {
        argv.push("--border-enable".to_owned());
        argv.push("--border-color-active".to_owned());
        argv.push(border.active_color.clone());
        argv.push("--border-color-inactive".to_owned());
        argv.push(border.inactive_color.clone());
        argv.push("--border-color-urgent".to_owned());
        argv.push(border.urgent_color.clone());
        argv.push("--border-thickness".to_owned());
        argv.push(border.thickness.to_string());

        if border.label.enable {
            let label = border.label.text.as_deref().unwrap_or(&input.vm_name);
            if !label.is_empty() {
                argv.push("--border-label".to_owned());
                argv.push(label.to_owned());
                argv.push("--border-label-position".to_owned());
                argv.push(border.label.position.as_str().to_owned());
            }
        }
    }

    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonzero(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("test thickness must be non-zero")
    }

    fn border_config() -> WaylandProxyBorderConfig {
        WaylandProxyBorderConfig {
            active_color: "#7fc8ff".to_owned(),
            inactive_color: "#45475a".to_owned(),
            urgent_color: "#f38ba8".to_owned(),
            thickness: nonzero(4),
            label: WaylandProxyBorderLabelConfig::default(),
        }
    }

    fn flag_value<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
        argv.iter()
            .position(|arg| arg == flag)
            .and_then(|idx| argv.get(idx + 1))
            .map(String::as_str)
    }

    #[test]
    fn default_argv_shape() {
        let input = WaylandProxyArgvInput::for_vm("work");
        let argv = generate_wayland_proxy_argv(&input).expect("valid input");
        assert_eq!(argv[0], "d2b-work-wlproxy", "argv[0] is process title");
        assert!(argv.contains(&"--listen".to_owned()));
        assert!(argv.contains(&"--connect".to_owned()));
        assert!(argv.contains(&"--vm-name".to_owned()));
        let listen_idx = argv.iter().position(|a| a == "--listen").unwrap();
        assert_eq!(argv[listen_idx + 1], "/run/d2b-wlproxy/work/wayland-0");
        let connect_idx = argv.iter().position(|a| a == "--connect").unwrap();
        assert_eq!(argv[connect_idx + 1], "/run/d2b-wlproxy/work/upstream");
    }

    #[test]
    fn default_argv_omits_border_flags() {
        let input = WaylandProxyArgvInput::for_vm("work");
        let argv = generate_wayland_proxy_argv(&input).expect("valid input");
        assert!(!argv.contains(&"--border-enable".to_owned()));
        assert!(flag_value(&argv, "--border-color-active").is_none());
        assert!(flag_value(&argv, "--border-thickness").is_none());
        assert!(flag_value(&argv, "--border-label").is_none());
        assert!(flag_value(&argv, "--border-label-position").is_none());
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
        assert_eq!(argv[prefix_idx + 1], "d2b.dev.");
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

    #[test]
    fn enabled_border_flags_include_colors_thickness_and_default_label() {
        let mut input = WaylandProxyArgvInput::for_vm("work");
        input.border = Some(border_config());

        let argv = generate_wayland_proxy_argv(&input).expect("valid");

        assert!(argv.contains(&"--border-enable".to_owned()));
        assert_eq!(flag_value(&argv, "--border-color-active"), Some("#7fc8ff"));
        assert_eq!(
            flag_value(&argv, "--border-color-inactive"),
            Some("#45475a")
        );
        assert_eq!(flag_value(&argv, "--border-color-urgent"), Some("#f38ba8"));
        assert_eq!(flag_value(&argv, "--border-thickness"), Some("4"));
        assert_eq!(flag_value(&argv, "--border-label"), Some("work"));
        assert_eq!(
            flag_value(&argv, "--border-label-position"),
            Some("top-left")
        );
    }

    #[test]
    fn custom_border_label_and_position_are_emitted() {
        let mut input = WaylandProxyArgvInput::for_vm("work");
        let mut border = border_config();
        border.label.text = Some("Work Desktop".to_owned());
        border.label.position = WaylandProxyBorderLabelPosition::TopCenter;
        input.border = Some(border);

        let argv = generate_wayland_proxy_argv(&input).expect("valid");

        assert_eq!(flag_value(&argv, "--border-label"), Some("Work Desktop"));
        assert_eq!(
            flag_value(&argv, "--border-label-position"),
            Some("top-center")
        );
    }

    #[test]
    fn disabled_border_label_omits_label_flags() {
        let mut input = WaylandProxyArgvInput::for_vm("work");
        let mut border = border_config();
        border.label.enable = false;
        input.border = Some(border);

        let argv = generate_wayland_proxy_argv(&input).expect("valid");

        assert!(argv.contains(&"--border-enable".to_owned()));
        assert!(flag_value(&argv, "--border-label").is_none());
        assert!(flag_value(&argv, "--border-label-position").is_none());
    }
}
