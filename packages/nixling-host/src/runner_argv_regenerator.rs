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

use crate::ch_argv::{generate_ch_argv, ChArgvInput};

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
/// not yet carry as typed data. v1.1.1 will add these to the
/// `processes.json` schema; at v1.1 the broker constructs them at
/// spawn time from the resolved bundle + host state.
#[derive(Debug, Clone)]
pub struct RunnerArgvExtra {
    /// Pre-built [`ChArgvInput`] when the role is
    /// [`ProcessRole::CloudHypervisorRunner`]. Mandatory for that
    /// role; ignored otherwise.
    pub ch_input: Option<ChArgvInput>,
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
            let ch_input =
                extra
                    .ch_input
                    .as_ref()
                    .ok_or_else(|| RegenerateArgvError::MissingInput {
                        role: intent.role.clone(),
                        field: "ch_input".to_owned(),
                    })?;
            let mut argv = generate_ch_argv(ch_input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            // SpawnRunnerPlanInput convention: argv[0] is the
            // process title (`microvm@<vm>` historically), NOT the
            // CH binary path. The binary path lives separately in
            // ResolvedRunnerIntent::binary_path. generate_ch_argv
            // emits argv[0] = ch_binary_path for execve(2) parity;
            // we replace it with the resolved-intent arg0 so the
            // broker's SpawnRunnerPlanInput shape matches.
            if !argv.is_empty() {
                argv[0] = intent
                    .argv
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("microvm@{}", intent.vm_name));
            }
            Ok(argv)
        }
        // All other roles already have dedicated Rust generators in
        // sibling modules (virtiofsd_argv, gpu_argv, audio_argv,
        // swtpm_argv, usbip_argv, video_argv, vsock_relay_argv,
        // otel_host_bridge_argv). v1.1.1 wires each through this
        // dispatcher; at v1.1 the canonical generators are exposed
        // but per-role regeneration arms remain stubs.
        other => Err(RegenerateArgvError::NotYetWired(other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_intent(role: ProcessRole) -> ResolvedRunnerIntent {
        ResolvedRunnerIntent {
            intent_id: "fake".to_owned(),
            vm_name: "fake-vm".to_owned(),
            role_id: "fake-role".to_owned(),
            role,
            binary_path: std::path::PathBuf::from("/bin/fake"),
            argv: vec!["fake".to_owned()],
            env: vec![],
            uid: 0,
            gid: 0,
            supplementary_groups: vec![],
            capabilities: vec![],
            namespaces: nixling_core::minijail_profile::NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: nixling_core::minijail_profile::MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: false,
            },
            cgroup_placement: nixling_core::minijail_profile::CgroupPlacement {
                subtree: "nixling.slice/test".to_owned(),
                controllers: vec![],
                delegated: false,
            },
            root_carve_out: false,
            profile_id: "test".to_owned(),
        }
    }

    #[test]
    fn ch_role_without_input_errors_with_missing_field() {
        let intent = fake_intent(ProcessRole::CloudHypervisorRunner);
        let err = regenerate_argv(&intent, &RunnerArgvExtra { ch_input: None }).unwrap_err();
        match err {
            RegenerateArgvError::MissingInput { field, .. } => assert_eq!(field, "ch_input"),
            other => panic!("expected MissingInput, got {other:?}"),
        }
    }

    #[test]
    fn non_ch_role_returns_not_yet_wired() {
        for role in [
            ProcessRole::Virtiofsd,
            ProcessRole::Swtpm,
            ProcessRole::Gpu,
            ProcessRole::Audio,
        ] {
            let intent = fake_intent(role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra { ch_input: None }).unwrap_err();
            assert!(
                matches!(err, RegenerateArgvError::NotYetWired(ref r) if r == &role),
                "expected NotYetWired({role:?}), got {err:?}"
            );
        }
    }
}
