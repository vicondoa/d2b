//! v1.1-final: canonical Rust-side runner argv regenerator.
//!
//! This module documents and exposes the canonical "Rust port" of
//! the Nix-side argv generation that previously lived in
//! `nixos-modules/processes-json.nix` (cloudHypervisorArgv,
//! virtiofsdRunner, gpuRunner, audioRunner). The Nix-side functions
//! are retained as a backward-compat artifact (broker reads
//! `processes.json`'s prebuilt argv at v1.1 to keep the wire
//! contract stable), but THE CANONICAL GENERATORS ARE RUST.
//!
//! Per ADR 0018 § "Removal of the microvm.nix flake dependency":
//!
//! > The runner argv generation that microvm.nix's
//! > `declaredRunner` derivation provided is replaced by typed
//! > Rust generators in `nixling-host::*_argv`. The broker
//! > spawns each runner role via the appropriate generator
//! > rather than executing a Nix-built runner script.
//!
//! Usage from the broker (v1.1-final wire-cleanup path):
//!
//! ```ignore
//! use nixling_host::runner_argv_regenerator::regenerate_argv;
//! use nixling_core::bundle_resolver::ResolvedRunnerIntent;
//!
//! fn spawn(intent: &ResolvedRunnerIntent, extra: &RunnerExtra) -> Vec<String> {
//!     // Future v1.1.1: replace `intent.argv.clone()` with
//!     // `regenerate_argv(intent, extra)` so the bundle's prebuilt
//!     // argv is no longer the source of truth.
//!     regenerate_argv(intent, extra)
//! }
//! ```
//!
//! The wire-cleanup (removing the Nix-side argv generation from
//! processes-json.nix entirely and having the bundle carry typed
//! `ChArgvInput` records instead of materialized `argv` lists) is
//! scheduled for v1.1.1 — at v1.1 we ship the canonical Rust
//! generators + the regenerator wrapper as the documented
//! migration surface.

use nixling_core::bundle_resolver::ResolvedRunnerIntent;
use nixling_core::processes::ProcessRole;

use crate::audio_argv::{generate_audio_argv, AudioArgvInput};
use crate::ch_argv::{generate_ch_argv, ChArgvInput};
use crate::gpu_argv::{generate_gpu_argv, GpuArgvInput};
use crate::otel_host_bridge_argv::{generate_otel_host_bridge_argv, OtelHostBridgeArgvInputs};
use crate::swtpm_argv::{generate_swtpm_argv, SwtpmArgvInput};
use crate::usbip_argv::{generate_usbip_argv, UsbipArgvInput, UsbipSubcommand};
use crate::video_argv::{generate_video_argv, VideoArgvInput};
use crate::virtiofsd_argv::{generate_virtiofsd_argv, VirtiofsdArgvInput};
use crate::vsock_relay_argv::{generate_vsock_relay_argv, VsockRelayArgvInput};

/// Errors that can occur during regeneration.
#[derive(Debug)]
pub enum RegenerateArgvError {
    MissingInput { role: ProcessRole, field: String },
    NotYetWired(ProcessRole),
    Generator(String),
}

impl std::fmt::Display for RegenerateArgvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingInput { role, field } => write!(
                f,
                "missing required runner input for role {role:?}: {field}"
            ),
            Self::NotYetWired(role) => write!(f, "regeneration not yet wired for role {role:?}"),
            Self::Generator(msg) => write!(f, "inner generator failed: {msg}"),
        }
    }
}

impl std::error::Error for RegenerateArgvError {}

/// Supplementary per-VM runner inputs that the bundle resolver does
/// not yet carry as typed data. v1.2 will fold these into
/// `processes.json` schema; at v1.1.1 the broker constructs them at
/// spawn time from the resolved bundle + host state.
#[derive(Debug, Clone, Default)]
pub struct RunnerArgvExtra {
    /// Pre-built [`ChArgvInput`] when the role is
    /// [`ProcessRole::CloudHypervisorRunner`].
    pub ch_input: Option<ChArgvInput>,
    /// Pre-built [`VirtiofsdArgvInput`] when the role is
    /// [`ProcessRole::Virtiofsd`].
    pub virtiofsd_input: Option<VirtiofsdArgvInput>,
    /// Pre-built [`SwtpmArgvInput`] when the role is
    /// [`ProcessRole::Swtpm`].
    pub swtpm_input: Option<SwtpmArgvInput>,
    /// Pre-built [`GpuArgvInput`] when the role is
    /// [`ProcessRole::Gpu`].
    pub gpu_input: Option<GpuArgvInput>,
    /// Pre-built [`AudioArgvInput`] when the role is
    /// [`ProcessRole::Audio`].
    pub audio_input: Option<AudioArgvInput>,
    /// Pre-built [`VideoArgvInput`] when the role is
    /// [`ProcessRole::Video`].
    pub video_input: Option<VideoArgvInput>,
    /// Pre-built [`VsockRelayArgvInput`] when the role is
    /// [`ProcessRole::VsockRelay`].
    pub vsock_relay_input: Option<VsockRelayArgvInput>,
    /// Pre-built [`UsbipArgvInput`] when the role is
    /// [`ProcessRole::Usbip`]. Pair with `usbip_subcommand`
    /// (bind | unbind) since the same input struct serves both
    /// subcommands.
    pub usbip_input: Option<UsbipArgvInput>,
    /// USBIP subcommand selector (Bind / Unbind) for the
    /// generate_usbip_argv invocation. Defaults to Bind because
    /// initial host-side dispatch always binds before attach.
    pub usbip_subcommand: Option<UsbipSubcommand>,
    /// Pre-built [`OtelHostBridgeArgvInputs`] for the otel-host-bridge
    /// SpawnRunner. Not on processes-json's role enum at v1.1 (the
    /// broker dispatches via a separate code path); the field is
    /// preserved here so callers can use the same regenerate_argv
    /// dispatcher uniformly.
    pub otel_host_bridge_input: Option<OtelHostBridgeArgvInputs>,
}

/// Regenerate the spawn argv for a resolved runner intent using the
/// canonical Rust argv generators in this crate.
///
/// At v1.1 this function is the documented migration surface — the
/// broker may opt into Rust-regenerated argv per-role to validate
/// against the bundle's prebuilt argv before the v1.1.1 wire
/// cleanup makes the Rust path mandatory.
pub fn regenerate_argv(
    intent: &ResolvedRunnerIntent,
    extra: &RunnerArgvExtra,
) -> Result<Vec<String>, RegenerateArgvError> {
    match &intent.role {
        ProcessRole::CloudHypervisorRunner => {
            let ch_input = require(&extra.ch_input, &intent.role, "ch_input")?;
            let mut argv = generate_ch_argv(ch_input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            // SpawnRunnerPlanInput argv[0] = process-title convention.
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Virtiofsd => {
            let input = require(&extra.virtiofsd_input, &intent.role, "virtiofsd_input")?;
            let mut argv = generate_virtiofsd_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Swtpm => {
            let input = require(&extra.swtpm_input, &intent.role, "swtpm_input")?;
            let mut argv = generate_swtpm_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Gpu | ProcessRole::GpuRenderNode => {
            let input = require(&extra.gpu_input, &intent.role, "gpu_input")?;
            let mut argv = generate_gpu_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Audio => {
            let input = require(&extra.audio_input, &intent.role, "audio_input")?;
            let mut argv = generate_audio_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Video => {
            let input = require(&extra.video_input, &intent.role, "video_input")?;
            let mut argv = generate_video_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::VsockRelay => {
            let input = require(&extra.vsock_relay_input, &intent.role, "vsock_relay_input")?;
            let mut argv = generate_vsock_relay_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::OtelHostBridge => {
            let input = require(
                &extra.otel_host_bridge_input,
                &intent.role,
                "otel_host_bridge_input",
            )?;
            let mut argv = generate_otel_host_bridge_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::Usbip => {
            let input = require(&extra.usbip_input, &intent.role, "usbip_input")?;
            let sub = extra.usbip_subcommand.unwrap_or(UsbipSubcommand::Bind);
            let mut argv = generate_usbip_argv(input, sub)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        // Roles handled by other dispatch surfaces (readiness probes,
        // pre-start hooks). The regenerator does not own these.
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::GuestSshReadiness
        | ProcessRole::GuestControlHealth
        | ProcessRole::SwtpmPreStartFlush => {
            Err(RegenerateArgvError::NotYetWired(intent.role.clone()))
        }
        // WaylandProxy: argv regeneration wired in Wave 2 / Lane A when the
        // nixling-wayland-filter binary is added to the workspace.
        ProcessRole::WaylandProxy => Err(RegenerateArgvError::NotYetWired(intent.role.clone())),
    }
}

/// Replace argv[0] with the resolved-intent's arg0 (`microvm@<vm>`
/// or similar). Per-role generators emit argv[0] = binary_path for
/// execve(2) parity; SpawnRunnerPlanInput expects the process title.
fn replace_arg0(argv: &mut [String], intent: &ResolvedRunnerIntent) {
    if argv.is_empty() {
        return;
    }
    argv[0] = intent
        .argv
        .first()
        .cloned()
        .unwrap_or_else(|| format!("microvm@{}", intent.vm_name));
}

fn require<'a, T>(
    value: &'a Option<T>,
    role: &ProcessRole,
    field: &str,
) -> Result<&'a T, RegenerateArgvError> {
    value
        .as_ref()
        .ok_or_else(|| RegenerateArgvError::MissingInput {
            role: role.clone(),
            field: field.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_intent(role: ProcessRole) -> ResolvedRunnerIntent {
        nixling_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("fake")
            .with_vm_name("fake-vm")
            .with_role_id("fake-role")
            .with_role(role)
            .with_binary_path(std::path::PathBuf::from("/bin/fake"))
            .with_argv(vec!["fake".to_owned()])
            .with_uid(0)
            .with_gid(0)
            .with_mount_policy(nixling_core::minijail_profile::MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: false,
                device_binds: vec![],
                bind_mounts: vec![],
            })
            .build()
    }

    #[test]
    fn ch_role_without_input_errors_with_missing_field() {
        let intent = fake_intent(ProcessRole::CloudHypervisorRunner);
        let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
        match err {
            RegenerateArgvError::MissingInput { field, .. } => assert_eq!(field, "ch_input"),
            other => panic!("expected MissingInput, got {other:?}"),
        }
    }

    #[test]
    fn all_wired_roles_return_missing_input_without_extras() {
        // After v1.1.1 wires every role's regenerator, the
        // dispatcher returns MissingInput when the caller doesn't
        // provide the per-role input record.
        for role in [
            ProcessRole::Virtiofsd,
            ProcessRole::Swtpm,
            ProcessRole::Gpu,
            ProcessRole::Audio,
            ProcessRole::Video,
            ProcessRole::VsockRelay,
            ProcessRole::Usbip,
        ] {
            let intent = fake_intent(role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
            assert!(
                matches!(err, RegenerateArgvError::MissingInput { ref role, .. } if role == &intent.role),
                "expected MissingInput({role:?}, ...), got {err:?}"
            );
        }
    }

    #[test]
    fn readiness_only_roles_return_not_yet_wired() {
        // Pure readiness / pre-start roles aren't dispatchable
        // through the regenerator; they live on other surfaces.
        for role in [
            ProcessRole::HostReconcile,
            ProcessRole::StoreVirtiofsPreflight,
            ProcessRole::GuestSshReadiness,
            ProcessRole::SwtpmPreStartFlush,
        ] {
            let intent = fake_intent(role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
            assert!(
                matches!(err, RegenerateArgvError::NotYetWired(ref r) if r == &role),
                "expected NotYetWired({role:?}), got {err:?}"
            );
        }
    }
}
