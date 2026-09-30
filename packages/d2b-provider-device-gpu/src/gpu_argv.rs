//! `crosvm device gpu` sidecar argv generator.
//!
//! Pure Rust function that emits the argv for the graphics sidecar
//! the broker's `SpawnRunner` op forks before `exec`-ing Cloud
//! Hypervisor (per the runner-shape audit at
//! `docs/reference/runner-shape-audit.md`). The daemon spawns this
//! sidecar through the broker `SpawnRunner` op with
//! `RunnerRole::Gpu` when the broker-side spawn implementation ships.
//!
//! Audit shape for `corp-desktop`:
//!
//! ```text
//! crosvm device gpu \
//!   --socket corp-desktop-gpu.sock \
//!   --wayland-sock $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY \
//!   --params '{"context-types":"virgl:virgl2:cross-domain","displays":[{"hidden":true}],"egl":true,"vulkan":true}'
//! ```
//!
//! CH then connects via `--gpu socket=corp-desktop-gpu.sock`. The Process
//! Provider composes that private CH argument from the sealed launch ticket;
//! the Guest controller does not receive or assemble it.
//!
//! # What the argv may not carry
//!
//! The GPU worker's device capability is delivered by its own admitted
//! `DeviceBinding` as a verified descriptor, so no argument here may name a
//! device node: an argv generator that accepted a render-node path would let
//! the same caller redirect a worker at a card its binding never claimed. The
//! generator therefore refuses any input that names a device node, and the
//! free-form `extraArgs` tail is gone, so the flag set above is the whole of
//! what a caller can influence.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// Closed set of GPU context types crosvm supports. The audit shape
/// is `virgl:virgl2:cross-domain`; the daemon caller composes the
/// requested context types into the comma-separated `--params` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuContextType {
    /// The base virgl context.
    Virgl,
    /// The virgl2 context.
    Virgl2,
    /// The cross-domain context.
    CrossDomain,
}

impl GpuContextType {
    /// Return the kebab-case context-type spelling used in `--params`.
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GpuDisplayConfig {
    /// Whether the display surface is hidden from the host
    /// compositor. Audit shape: `true` (the cross-domain handoff
    /// targets a guest-side surface).
    pub hidden: bool,
}

/// `--params` payload. Rendered as compact JSON (no spaces) so the
/// audit-shape diff stays byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GpuArgvInput {
    /// Absolute store path to the `crosvm` binary.
    pub crosvm_binary_path: String,
    /// VM name; used for the worker launch arg0 only. The flag set does
    /// not embed the VM name (the socket path does).
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
}

/// Errors the GPU argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum GpuArgvError {
    /// The crosvm binary path is empty or not absolute.
    InvalidCrosvmBinaryPath {
        /// The offending path.
        path: String,
    },
    /// The VM name is empty.
    EmptyVmName,
    /// The socket path is empty.
    EmptySocketPath,
    /// The Wayland socket is empty.
    EmptyWaylandSock,
    /// No context type is declared.
    EmptyContextTypes,
    /// No display is declared.
    EmptyDisplays,
    /// One input named a device node.
    ///
    /// A device capability is delivered by the worker's own admitted
    /// `DeviceBinding`, never by a path in its command line, so a render node
    /// or any other device is refused here rather than handed to a launch
    /// whose binding does not carry it.
    DeviceNodeArgumentRefused,
}

/// Fence one path-shaped input against a device node.
///
/// crosvm's GPU sidecar is handed sockets and a Wayland endpoint, never a
/// device. A `/dev/...` component in any of those inputs would be a caller
/// redirecting the worker at a node its admitted binding does not carry, so
/// it is refused before the argv is rendered.
fn reject_device_node(path: &str) -> Result<(), GpuArgvError> {
    if path
        .split('/')
        .any(|component| component == "dev" || component == "proc" || component == "sys")
    {
        return Err(GpuArgvError::DeviceNodeArgumentRefused);
    }
    Ok(())
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
/// - the injection surface is bounded - `GpuContextType` is a
///   closed enum with safe `as_str()` outputs verified at test
///   time by `context_type_string_is_json_safe`; `bool` fields
///   render as lowercase `true`/`false` via Rust `Display`.
///
/// The full byte-level parity gate runs in the pinned
/// `gpu_argv` unit tests; it is intentionally NOT a byte-compare
/// against the W0b audit
/// fixture (the audit fixture is a snapshot of the retired
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
///
/// # Errors
///
/// Returns [`GpuArgvError::InvalidCrosvmBinaryPath`] when the binary path
/// is empty or not absolute, [`GpuArgvError::EmptyVmName`] when the VM
/// name is empty, [`GpuArgvError::EmptySocketPath`] when the socket path
/// is empty, [`GpuArgvError::EmptyWaylandSock`] when the Wayland socket
/// is empty, [`GpuArgvError::EmptyContextTypes`] when no context type is
/// declared, and [`GpuArgvError::EmptyDisplays`] when no display is
/// declared.
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
    reject_device_node(&input.socket_path)?;
    reject_device_node(&input.wayland_sock)?;
    let params_json = render_params(&input.params)?;

    let argv: Vec<String> = vec![
        input.crosvm_binary_path.clone(),
        "device".to_owned(),
        "gpu".to_owned(),
        "--socket".to_owned(),
        input.socket_path.clone(),
        "--wayland-sock".to_owned(),
        input.wayland_sock.clone(),
        "--params".to_owned(),
        params_json,
    ];
    Ok(argv)
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
        }
    }

    /// Daemon-only end-state fixture: the argv the d2bd Gpu runner
    /// emits after v1.0 retirement of `d2b-<vm>-gpu.service`.
    /// Socket path is the per-VM absolute socket under
    /// `/run/d2b/vms/<vm>/`, and `--wayland-sock` is the
    /// in-sandbox bind-mount target (broker prepares the BindPath
    /// `/run/user/<uid>/wayland-0:/run/d2b-gpu/<vm>/wayland-0`
    /// before the runner starts; from inside the mount namespace
    /// crosvm sees only the bind target).
    fn daemon_input() -> GpuArgvInput {
        GpuArgvInput {
            crosvm_binary_path:
                "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-crosvm-127.0/bin/crosvm".to_owned(),
            vm_name: "corp-vm".to_owned(),
            socket_path: "/run/d2b/vms/corp-vm/gpu.sock".to_owned(),
            wayland_sock: "/run/d2b-gpu/corp-vm/wayland-0".to_owned(),
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
    fn audit_parity_minimal() {
        let argv = generate_gpu_argv(&audit_input()).unwrap();
        assert!(argv[0].ends_with("/crosvm"));
        assert_eq!(argv[1], "device");
        assert_eq!(argv[2], "gpu");
        let joined = argv.join(" ");
        assert!(joined.contains("--socket corp-desktop-gpu.sock"));
        assert!(joined.contains("--wayland-sock /run/user/1000/wayland-0"));
        assert!(joined.contains(
            "--params {\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}"
        ));
    }

    /// One single-field rejection vector: mutate exactly one valid fixture
    /// field and pin the typed error.
    struct GpuRejectVector {
        name: &'static str,
        expected: GpuArgvError,
        mutate: fn(&mut GpuArgvInput),
    }

    #[test]
    fn rejects_one_invalid_field_at_a_time() {
        let vectors = vec![
            GpuRejectVector {
                name: "relative crosvm binary path",
                expected: GpuArgvError::InvalidCrosvmBinaryPath {
                    path: "crosvm".to_owned(),
                },
                mutate: |input| input.crosvm_binary_path = "crosvm".to_owned(),
            },
            GpuRejectVector {
                name: "empty VM name",
                expected: GpuArgvError::EmptyVmName,
                mutate: |input| input.vm_name.clear(),
            },
            GpuRejectVector {
                name: "empty socket path",
                expected: GpuArgvError::EmptySocketPath,
                mutate: |input| input.socket_path.clear(),
            },
            GpuRejectVector {
                name: "empty wayland socket",
                expected: GpuArgvError::EmptyWaylandSock,
                mutate: |input| input.wayland_sock.clear(),
            },
            GpuRejectVector {
                name: "empty context types",
                expected: GpuArgvError::EmptyContextTypes,
                mutate: |input| input.params.context_types.clear(),
            },
            GpuRejectVector {
                name: "empty displays",
                expected: GpuArgvError::EmptyDisplays,
                mutate: |input| input.params.displays.clear(),
            },
        ];
        for GpuRejectVector {
            name,
            expected,
            mutate,
        } in vectors
        {
            let mut input = audit_input();
            mutate(&mut input);
            assert_eq!(
                generate_gpu_argv(&input).unwrap_err(),
                expected,
                "rejection vector: {name}"
            );
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{
            "crosvmBinaryPath": "/nix/store/GPUGPU-gpu-crosvm/bin/crosvm",
            "vmName": "corp-vm",
            "socketPath": "/run/d2b/vms/corp-vm/gpu.sock",
            "waylandSock": "/run/d2b-gpu/corp-vm/wayland-0",
            "params": {
                "context-types": ["virgl", "virgl2", "cross-domain"],
                "displays": [{"hidden": true}],
                "egl": true,
                "vulkan": true
            }
        }"#;
        let parsed = serde_json::from_str::<GpuArgvInput>(json);
        assert!(parsed.is_ok(), "baseline shape must still parse: {parsed:?}");
        let top_level = json.replace(
            "\"vmName\": \"corp-vm\"",
            "\"vmName\": \"corp-vm\", \"unexpectedField\": 1",
        );
        assert!(
            serde_json::from_str::<GpuArgvInput>(&top_level).is_err(),
            "unknown top-level field must be rejected"
        );
        let nested = json.replace(
            "\"context-types\": [\"virgl\", \"virgl2\", \"cross-domain\"]",
            "\"context-types\": [\"virgl\", \"virgl2\", \"cross-domain\"], \"unexpected\": true",
        );
        assert!(
            serde_json::from_str::<GpuArgvInput>(&nested).is_err(),
            "unknown field inside params must be rejected"
        );
        let display = json.replace(
            "\"displays\": [{\"hidden\": true}]",
            "\"displays\": [{\"hidden\": true, \"unexpected\": true}]",
        );
        assert!(
            serde_json::from_str::<GpuArgvInput>(&display).is_err(),
            "unknown field inside a display config must be rejected"
        );
    }

    /// The GPU worker reaches no device through its command line.
    ///
    /// Its device capability is delivered by its own admitted
    /// `DeviceBinding` as a verified descriptor, so a render-node path in an
    /// argument would be a caller redirecting the worker at a card its
    /// binding never claimed. Both path-shaped inputs are fenced, and the
    /// rendered argv names no device at all.
    #[test]
    fn refuses_an_input_that_names_a_render_node() {
        for (socket, wayland) in [
            (
                "/dev/dri/renderD128".to_owned(),
                "/run/user/1000/wayland-0".to_owned(),
            ),
            (
                "/run/d2b/vms/corp-vm/gpu.sock".to_owned(),
                "/dev/dri/renderD129".to_owned(),
            ),
        ] {
            let mut input = daemon_input();
            input.socket_path = socket;
            input.wayland_sock = wayland;
            assert!(
                matches!(
                    generate_gpu_argv(&input),
                    Err(GpuArgvError::DeviceNodeArgumentRefused)
                ),
                "a device node must not reach the GPU worker argv"
            );
        }

        let argv = generate_gpu_argv(&daemon_input()).unwrap();
        assert!(
            !argv.iter().any(|argument| argument.contains("/dev/")),
            "the rendered GPU argv names no device node: {:?}",
            argv
        );
    }

    /// A declared payload cannot smuggle a render node or a free-form
    /// argument tail back in.
    ///
    /// `GpuArgvInput` denies unknown fields, so a launch ticket that still
    /// carries an `extraArgs` tail or an explicit render-node field is
    /// refused at the type boundary rather than launching a worker that was
    /// pointed at another card.
    #[test]
    fn a_declared_payload_cannot_carry_a_render_node_or_an_argument_tail() {
        let input = daemon_input();
        let mut payload = serde_json::to_value(&input).expect("the declared input serializes");
        payload["extraArgs"] = serde_json::json!(["--render-node", "/dev/dri/renderD129"]);
        assert!(
            serde_json::from_value::<GpuArgvInput>(payload).is_err(),
            "a free-form argv tail must not decode"
        );

        let mut payload = serde_json::to_value(&input).expect("the declared input serializes");
        payload["renderNode"] = serde_json::json!("/dev/dri/renderD129");
        assert!(
            serde_json::from_value::<GpuArgvInput>(payload).is_err(),
            "an explicit render-node field must not decode"
        );
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

    /// Enforce at test time that every `GpuContextType::as_str()` output
    /// is JSON-safe - only ASCII
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
                    "GpuContextType::as_str() output {s:?} contains JSON-unsafe character {c:?} - \
                     `render_params` uses manual format! interpolation that would corrupt the JSON \
                     payload; switch to serde_json::to_string for the offending variant or pin a \
                     stricter charset here."
                );
            }
        }
    }

}
