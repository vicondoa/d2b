//! vhost-device-sound audio sidecar argv generator.
//!
//! Pure Rust function that emits the argv for the per-VM
//! `nixling-<vm>-snd.service` audio sidecar per
//! `nixos-modules/components/audio/host.nix`. The sidecar publishes
//! the guest's virtio-snd backend to the host's PipeWire session;
//! see `docs/reference/components-audio.md` for the full ACL +
//! mediation contract.
//!
//! Shape:
//!
//! ```text
//! /run/nixling/vms/<vm>/nixling-<vm> \
//!   --socket /run/nixling/vms/<vm>/snd.sock \
//!   --backend pipewire
//! ```
//!
//! `argv[0]` is the per-VM copy of `vhost-device-sound` the
//! `ExecStartPre` `install`s so libpipewire's `init_prgname()`
//! derives `application.name = "nixling-<vm>"` from `/proc/self/exe`
//! (per audio component docs §"Host-side resources created").
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// Audio backend the sidecar speaks to the host. Today only PipeWire
/// is supported per `nixos-modules/components/audio/host.nix`; the
/// enum keeps the wire stable for future backends (`alsa`, `null`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioBackend {
    Pipewire,
}

impl AudioBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pipewire => "pipewire",
        }
    }
}

/// All inputs required to render the vhost-device-sound argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioArgvInput {
    /// Absolute path to the per-VM `vhost-device-sound` copy that
    /// the audio sidecar's `ExecStartPre` installs at
    /// `/run/nixling/vms/<vm>/nixling-<vm>`.
    pub sidecar_binary_path: String,
    /// VM name; used by [`exec_arg0`] only.
    pub vm_name: String,
    /// `--socket` value. Per docs:
    /// `/run/nixling/vms/<vm>/snd.sock`. The audio component module
    /// asserts the parent dir is `RuntimeDirectory = nixling/vms/<vm>`
    /// with mode 0700 owned by `nixling-<vm>-snd`.
    pub socket_path: String,
    /// `--backend` value.
    pub backend: AudioBackend,
    /// Free-form additional vhost-device-sound args. Caller is
    /// responsible for quoting; each entry is emitted as-is in order
    /// at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Errors the audio argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum AudioArgvError {
    InvalidSidecarBinaryPath {
        path: String,
    },
    EmptyVmName,
    EmptySocketPath,
    /// The audio sidecar binary path MUST be the per-VM copy at
    /// `/run/nixling/vms/<vm>/nixling-<vm>` (so libpipewire's
    /// `init_prgname()` derives `application.name = "nixling-<vm>"`
    /// from `/proc/self/exe`). Any other path — including a
    /// Nix-store direct path, another VM's per-VM copy, a
    /// `current-system` symlink that resolves into the store, or
    /// any other absolute executable — defeats the
    /// stream-tracking design.
    SidecarBinaryPathNotPerVmCopy {
        path: String,
        expected: String,
    },
}

/// Enforce the EXACT per-VM-copy path shape rather than the shallow
/// "no /nix/store/" denylist. The broker / sidecar installer guarantees
/// that this path exists
/// as a root-owned regular file at spawn time; this generator
/// refuses any other shape.
fn expected_audio_sidecar_path(vm_name: &str) -> String {
    format!("/run/nixling/vms/{vm_name}/nixling-{vm_name}")
}

/// Render the audio sidecar argv.
pub fn generate_audio_argv(input: &AudioArgvInput) -> Result<Vec<String>, AudioArgvError> {
    if input.sidecar_binary_path.is_empty() || !input.sidecar_binary_path.starts_with('/') {
        return Err(AudioArgvError::InvalidSidecarBinaryPath {
            path: input.sidecar_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(AudioArgvError::EmptyVmName);
    }
    // Strict per-VM-copy path enforcement.
    let expected = expected_audio_sidecar_path(&input.vm_name);
    if input.sidecar_binary_path != expected {
        return Err(AudioArgvError::SidecarBinaryPathNotPerVmCopy {
            path: input.sidecar_binary_path.clone(),
            expected,
        });
    }
    if input.socket_path.is_empty() {
        return Err(AudioArgvError::EmptySocketPath);
    }
    let mut argv: Vec<String> = vec![
        input.sidecar_binary_path.clone(),
        "--socket".to_owned(),
        input.socket_path.clone(),
        "--backend".to_owned(),
        input.backend.as_str().to_owned(),
    ];
    for extra in &input.extra_args {
        argv.push(extra.clone());
    }
    Ok(argv)
}

/// `arg0` for the audio sidecar. Matches the systemd unit name
/// `nixling-<vm>-snd` (per `nixos-modules/components/audio/host.nix`).
pub fn exec_arg0(input: &AudioArgvInput) -> Result<String, AudioArgvError> {
    if input.vm_name.is_empty() {
        return Err(AudioArgvError::EmptyVmName);
    }
    Ok(format!("nixling-{}-snd", input.vm_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_input() -> AudioArgvInput {
        AudioArgvInput {
            sidecar_binary_path: "/run/nixling/vms/corp-desktop/nixling-corp-desktop".to_owned(),
            vm_name: "corp-desktop".to_owned(),
            socket_path: "/run/nixling/vms/corp-desktop/snd.sock".to_owned(),
            backend: AudioBackend::Pipewire,
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn audit_parity_minimal() {
        let argv = generate_audio_argv(&audit_input()).unwrap();
        let joined = argv.join(" ");
        assert_eq!(
            argv[0],
            "/run/nixling/vms/corp-desktop/nixling-corp-desktop"
        );
        assert!(joined.contains("--socket /run/nixling/vms/corp-desktop/snd.sock"));
        assert!(joined.contains("--backend pipewire"));
    }

    /// Byte-parity snapshot printed for the `tests/audio-argv-shape.sh`
    /// golden gate against
    /// `tests/golden/runner-shape/audio-argv-minimal.txt`.
    #[test]
    fn audit_minimal_snapshot_line() {
        let argv = generate_audio_argv(&audit_input()).unwrap();
        println!("SNAPSHOT: {}", argv.join(" "));
    }

    #[test]
    fn exec_arg0_matches_systemd_unit_name() {
        assert_eq!(
            exec_arg0(&audit_input()).unwrap(),
            "nixling-corp-desktop-snd"
        );
    }

    #[test]
    fn rejects_non_absolute_binary() {
        let mut input = audit_input();
        input.sidecar_binary_path = "vhost-device-sound".to_owned();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::InvalidSidecarBinaryPath { .. })
        ));
    }

    /// Direct Nix-store paths must be refused because they bypass
    /// libpipewire's `application.name`
    /// derivation (see module docstring for rationale).
    #[test]
    fn rejects_nix_store_direct_path() {
        let mut input = audit_input();
        input.sidecar_binary_path =
            "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-vhost-device-sound/bin/vhost-device-sound"
                .to_owned();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::SidecarBinaryPathNotPerVmCopy { .. })
        ));
    }

    /// Even /run/current-system symlinks (which would canonicalize INTO
    /// /nix/store) are
    /// refused by the strict per-VM-copy match.
    #[test]
    fn rejects_run_current_system_symlink() {
        let mut input = audit_input();
        input.sidecar_binary_path = "/run/current-system/sw/bin/vhost-device-sound".to_owned();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::SidecarBinaryPathNotPerVmCopy { .. })
        ));
    }

    /// Another VM's per-VM copy is also refused — the binary path is
    /// keyed on the input's
    /// `vm_name`.
    #[test]
    fn rejects_other_vms_per_vm_copy() {
        let mut input = audit_input();
        input.sidecar_binary_path = "/run/nixling/vms/other/nixling-other".to_owned();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::SidecarBinaryPathNotPerVmCopy { .. })
        ));
    }

    #[test]
    fn rejects_empty_binary() {
        let mut input = audit_input();
        input.sidecar_binary_path.clear();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::InvalidSidecarBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_empty_socket_path() {
        let mut input = audit_input();
        input.socket_path.clear();
        assert!(matches!(
            generate_audio_argv(&input),
            Err(AudioArgvError::EmptySocketPath)
        ));
    }

    #[test]
    fn exec_arg0_rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            exec_arg0(&input),
            Err(AudioArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn extra_args_appended_in_order() {
        let mut input = audit_input();
        input.extra_args = vec!["--debug".to_owned()];
        let argv = generate_audio_argv(&input).unwrap();
        assert_eq!(argv.last().unwrap(), "--debug");
    }

    #[test]
    fn backend_string_round_trip() {
        assert_eq!(AudioBackend::Pipewire.as_str(), "pipewire");
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: AudioArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
