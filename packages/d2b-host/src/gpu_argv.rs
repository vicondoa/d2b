//! `crosvm device gpu` sidecar argv generator.
//!
//! Pure Rust function that emits the argv microvm.nix's graphics
//! runner forks inline before `exec`-ing Cloud Hypervisor (per the
//! runner-shape audit at `docs/reference/runner-shape-audit.md`).
//! The daemon spawns this sidecar through the broker `SpawnRunner` op
//! with `RunnerRole::Gpu` when the broker-side spawn implementation ships.
//!
//! Audit shape for `corp-desktop`:
//!
//! ```text
//! crosvm device gpu \
//!   --socket corp-desktop-gpu.sock \
//!   --wayland-sock $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY \
//!   --gpu-device-node /proc/self/fd/10 \
//!   --params '{"context-types":"virgl:virgl2:cross-domain","displays":[{"hidden":true}],"egl":true,"vulkan":true}'
//! ```
//!
//! CH then connects via `--gpu socket=corp-desktop-gpu.sock` — that
//! flag is appended by the daemon caller into
//! [`crate::ch_argv::ChArgvInput::extra_args`] when assembling the
//! graphics VM's CH argv.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

pub const GPU_DEVICE_FD_PATH: &str = "/proc/self/fd/10";

/// Closed set of GPU context types crosvm supports. The audit shape
/// is `virgl:virgl2:cross-domain`; the daemon caller composes the
/// requested context types into the comma-separated `--params` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuContextType {
    Virgl,
    Virgl2,
    CrossDomain,
}

impl GpuContextType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Virgl => "virgl",
            Self::Virgl2 => "virgl2",
            Self::CrossDomain => "cross-domain",
        }
    }
}

/// Display config; one entry per virtual display. The audit shape is
/// `[{"hidden":true}]` (single hidden display).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDisplayConfig {
    /// Whether the display surface is hidden from the host
    /// compositor. Audit shape: `true` (the cross-domain handoff
    /// targets a guest-side surface).
    pub hidden: bool,
}

/// `--params` payload. Rendered as compact JSON (no spaces) so the
/// audit-shape diff stays byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GpuParams {
    /// Colon-separated context types (`virgl:virgl2:cross-domain`).
    pub context_types: Vec<GpuContextType>,
    /// Virtual displays. Audit shape has one hidden display.
    pub displays: Vec<GpuDisplayConfig>,
    /// EGL rendering. Audit shape: `true`.
    pub egl: bool,
    /// Vulkan rendering. Audit shape: `true`.
    pub vulkan: bool,
}

/// All inputs required to render the `crosvm device gpu` argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuArgvInput {
    /// Absolute store path to the `crosvm` binary.
    pub crosvm_binary_path: String,
    /// VM name; used for [`exec_arg0`] only. The flag set does not
    /// embed the VM name (the socket path does).
    pub vm_name: String,
    /// `--socket` value. Audit uses runner-cwd-relative
    /// `<vm>-gpu.sock`; the daemon uses an absolute path under
    /// `/run/d2b/vms/<vm>/`. Either shape is honoured.
    pub socket_path: String,
    /// `--wayland-sock` value. Resolved by the daemon caller to the
    /// host's primary Wayland session socket (per `d2b.site.waylandUser`).
    pub wayland_sock: String,
    /// `--params` JSON payload.
    pub params: GpuParams,
    /// Free-form additional crosvm args. Caller is responsible for
    /// quoting; each entry is emitted as-is in order at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Errors the GPU argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum GpuArgvError {
    InvalidCrosvmBinaryPath { path: String },
    EmptyVmName,
    EmptySocketPath,
    EmptyWaylandSock,
    EmptyContextTypes,
    EmptyDisplays,
}

/// Render the params JSON payload. Compact (no spaces) so the
/// audit-shape diff stays byte-stable against
/// `tests/golden/runner-shape/`.
///
/// Implementation note: this uses manual `format!` rather than
/// `serde_json::to_string` because:
///
/// - the byte-stable parity diff vs the W0b audit fixture pins
///   the exact field order; serde_json::to_string does not
///   guarantee object-field ordering;
/// - the injection surface is bounded — `GpuContextType` is a
///   closed enum with safe `as_str()` outputs verified at test
///   time by `context_type_string_is_json_safe`; `bool` fields
///   render as lowercase `true`/`false` via Rust `Display`.
///
/// The full byte-level parity gate runs in the pinned
/// `gpu_argv` unit tests; it is intentionally NOT a byte-compare
/// against the W0b audit
/// fixture (the audit fixture is a snapshot of microvm.nix's
/// runner shape and includes a `${runtime_args:-}` template
/// expansion the daemon never emits).
fn render_params(params: &GpuParams) -> Result<String, GpuArgvError> {
    if params.context_types.is_empty() {
        return Err(GpuArgvError::EmptyContextTypes);
    }
    if params.displays.is_empty() {
        return Err(GpuArgvError::EmptyDisplays);
    }
    let context_types_csv = params
        .context_types
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(":");
    let displays_json = params
        .displays
        .iter()
        .map(|d| format!("{{\"hidden\":{}}}", d.hidden))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"context-types\":\"{context_types_csv}\",\"displays\":[{displays_json}],\"egl\":{},\"vulkan\":{}}}",
        params.egl, params.vulkan
    ))
}

/// Render the `crosvm device gpu` argv.
pub fn generate_gpu_argv(input: &GpuArgvInput) -> Result<Vec<String>, GpuArgvError> {
    if input.crosvm_binary_path.is_empty() || !input.crosvm_binary_path.starts_with('/') {
        return Err(GpuArgvError::InvalidCrosvmBinaryPath {
            path: input.crosvm_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(GpuArgvError::EmptyVmName);
    }
    if input.socket_path.is_empty() {
        return Err(GpuArgvError::EmptySocketPath);
    }
    if input.wayland_sock.is_empty() {
        return Err(GpuArgvError::EmptyWaylandSock);
    }
    let params_json = render_params(&input.params)?;

    let mut argv: Vec<String> = vec![
        input.crosvm_binary_path.clone(),
        "device".to_owned(),
        "gpu".to_owned(),
        "--socket".to_owned(),
        input.socket_path.clone(),
        "--wayland-sock".to_owned(),
        input.wayland_sock.clone(),
        "--gpu-device-node".to_owned(),
        GPU_DEVICE_FD_PATH.to_owned(),
        "--params".to_owned(),
        params_json,
    ];
    for extra in &input.extra_args {
        argv.push(extra.clone());
    }
    Ok(argv)
}

/// `arg0` the daemon passes to `execvp` so the process shows up in
/// `ps` as `d2b-<vm>-gpu`.
pub fn exec_arg0(input: &GpuArgvInput) -> Result<String, GpuArgvError> {
    if input.vm_name.is_empty() {
        return Err(GpuArgvError::EmptyVmName);
    }
    Ok(format!("d2b-{}-gpu", input.vm_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_input() -> GpuArgvInput {
        GpuArgvInput {
            crosvm_binary_path: "/nix/store/rfw2rn9875py1l34wfr45wnlkphbgj5n-crosvm/bin/crosvm"
                .to_owned(),
            vm_name: "corp-desktop".to_owned(),
            socket_path: "corp-desktop-gpu.sock".to_owned(),
            wayland_sock: "/run/user/1000/wayland-0".to_owned(),
            params: GpuParams {
                context_types: vec![
                    GpuContextType::Virgl,
                    GpuContextType::Virgl2,
                    GpuContextType::CrossDomain,
                ],
                displays: vec![GpuDisplayConfig { hidden: true }],
                egl: true,
                vulkan: true,
            },
            extra_args: Vec::new(),
        }
    }

    /// Realm-role fixture for the allocator-mediated GPU runner.
    fn daemon_input() -> GpuArgvInput {
        GpuArgvInput {
            crosvm_binary_path:
                "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-crosvm-127.0/bin/crosvm".to_owned(),
            vm_name: "ucvmyzodoxhnswumcjsa".to_owned(),
            socket_path: "/run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/lindrso5iyyzzdrghemq/gpu.sock".to_owned(),
            wayland_sock: "/run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/z4kayxxlmcmxc4m3s4ga/wayland-0".to_owned(),
            params: GpuParams {
                context_types: vec![
                    GpuContextType::Virgl,
                    GpuContextType::Virgl2,
                    GpuContextType::CrossDomain,
                ],
                displays: vec![GpuDisplayConfig { hidden: true }],
                egl: true,
                vulkan: true,
            },
            extra_args: Vec::new(),
        }
    }

    const GPU_ARGV_GOLDEN: &str =
        include_str!("../../../tests/golden/runner-shape/gpu-argv-minimal.txt");

    fn golden_payload() -> String {
        GPU_ARGV_GOLDEN
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn daemon_input_snapshot_line() {
        let argv = generate_gpu_argv(&daemon_input()).unwrap();
        let observed = argv.join(" ");
        let expected = golden_payload();
        assert_eq!(
            observed, expected,
            "gpu argv drifted from tests/golden/runner-shape/gpu-argv-minimal.txt"
        );
        println!("SNAPSHOT: {observed}");
    }

    #[test]
    fn daemon_input_pins_cross_domain_and_wayland_bind() {
        let argv = generate_gpu_argv(&daemon_input()).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("device gpu"));
        assert!(joined.contains(
            "--socket /run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/lindrso5iyyzzdrghemq/gpu.sock"
        ));
        assert!(joined.contains(
            "--wayland-sock /run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/z4kayxxlmcmxc4m3s4ga/wayland-0"
        ));
        assert!(joined.contains("--gpu-device-node /proc/self/fd/10"));
        assert!(joined.contains(
            "--params {\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}"
        ));
    }

    #[test]
    fn audit_parity_minimal() {
        let argv = generate_gpu_argv(&audit_input()).unwrap();
        assert!(argv[0].ends_with("/crosvm"));
        assert_eq!(argv[1], "device");
        assert_eq!(argv[2], "gpu");
        let joined = argv.join(" ");
        assert!(joined.contains("--socket corp-desktop-gpu.sock"));
        assert!(joined.contains("--wayland-sock /run/user/1000/wayland-0"));
        assert!(joined.contains("--gpu-device-node /proc/self/fd/10"));
        assert!(joined.contains(
            "--params {\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}"
        ));
    }

    #[test]
    fn exec_arg0_matches_systemd_unit_name() {
        assert_eq!(exec_arg0(&audit_input()).unwrap(), "d2b-corp-desktop-gpu");
    }

    #[test]
    fn rejects_invalid_binary_path() {
        let mut input = audit_input();
        input.crosvm_binary_path = "crosvm".to_owned();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::InvalidCrosvmBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_empty_socket_path() {
        let mut input = audit_input();
        input.socket_path.clear();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::EmptySocketPath)
        ));
    }

    #[test]
    fn rejects_empty_wayland_sock() {
        let mut input = audit_input();
        input.wayland_sock.clear();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::EmptyWaylandSock)
        ));
    }

    #[test]
    fn rejects_empty_context_types() {
        let mut input = audit_input();
        input.params.context_types.clear();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::EmptyContextTypes)
        ));
    }

    #[test]
    fn rejects_empty_displays() {
        let mut input = audit_input();
        input.params.displays.clear();
        assert!(matches!(
            generate_gpu_argv(&input),
            Err(GpuArgvError::EmptyDisplays)
        ));
    }

    #[test]
    fn extra_args_appended_in_order() {
        let mut input = audit_input();
        input.extra_args = vec![
            "--seccomp-policy-dir".to_owned(),
            "/etc/crosvm/seccomp".to_owned(),
        ];
        let argv = generate_gpu_argv(&input).unwrap();
        let last_two = &argv[argv.len() - 2..];
        assert_eq!(last_two, &["--seccomp-policy-dir", "/etc/crosvm/seccomp"]);
    }

    #[test]
    fn params_renders_multi_display() {
        let mut input = audit_input();
        input.params.displays = vec![
            GpuDisplayConfig { hidden: true },
            GpuDisplayConfig { hidden: false },
        ];
        let argv = generate_gpu_argv(&input).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("\"displays\":[{\"hidden\":true},{\"hidden\":false}]"));
    }

    #[test]
    fn params_renders_subset_context_types() {
        let mut input = audit_input();
        input.params.context_types = vec![GpuContextType::Virgl2];
        let argv = generate_gpu_argv(&input).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("\"context-types\":\"virgl2\""));
    }

    #[test]
    fn params_omits_egl_when_false() {
        let mut input = audit_input();
        input.params.egl = false;
        let argv = generate_gpu_argv(&input).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("\"egl\":false"));
    }

    #[test]
    fn context_type_string_round_trip() {
        let pairs = [
            (GpuContextType::Virgl, "virgl"),
            (GpuContextType::Virgl2, "virgl2"),
            (GpuContextType::CrossDomain, "cross-domain"),
        ];
        for (ct, expected) in pairs {
            assert_eq!(ct.as_str(), expected);
        }
    }

    /// Enforce at test time that every `GpuContextType::as_str()` output
    /// is JSON-safe — only ASCII
    /// letters / digits / dash / underscore allowed. If a future
    /// variant ships with a quote, backslash, comma, or control
    /// character, this test fails closed rather than silently
    /// corrupting the manually-rendered `--params` JSON.
    #[test]
    fn context_type_string_is_json_safe() {
        for ct in [
            GpuContextType::Virgl,
            GpuContextType::Virgl2,
            GpuContextType::CrossDomain,
        ] {
            let s = ct.as_str();
            for c in s.chars() {
                assert!(
                    c.is_ascii_alphanumeric() || c == '-' || c == '_',
                    "GpuContextType::as_str() output {s:?} contains JSON-unsafe character {c:?} — \
                     `render_params` uses manual format! interpolation that would corrupt the JSON \
                     payload; switch to serde_json::to_string for the offending variant or pin a \
                     stricter charset here."
                );
            }
        }
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: GpuArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
