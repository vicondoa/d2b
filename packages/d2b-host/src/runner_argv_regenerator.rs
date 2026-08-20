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
//! > Rust generators in the neutral host surface and owning Providers. The
//! > broker keeps the trusted bundle argv authoritative until a Provider launch
//! > ticket carries the typed inputs required for regeneration.
//!
//! Usage from the broker (v1.1-final wire-cleanup path):
//!
//! ```ignore
//! use d2b_host::runner_argv_regenerator::regenerate_argv;
//! use d2b_core::bundle_resolver::ResolvedRunnerIntent;
//!
//! fn spawn(intent: &ResolvedRunnerIntent, extra: &RunnerExtra) -> Vec<String> {
//!     // Future v1.1.1: replace `intent.argv.clone()` with
//!     // `regenerate_argv(intent, extra)` so the bundle's prebuilt
//!     // argv is no longer the source of truth.
//!     regenerate_argv(intent, extra)
//! }
//! ```
//!
//! Provider-specific regeneration is intentionally retired here. This module
//! only validates neutral generators whose inputs are available to the host
//! surface; runtime Providers remain bundle-authoritative.
//!
//! Audio is intentionally excluded from this generic host regenerator. Its
//! argv builder is Provider-owned; until the runtime wave seals a typed
//! bundle projection, the bundle's argv remains authoritative and Audio
//! returns [`RegenerateArgvError::NotYetWired`].

use d2b_core::bundle_resolver::ResolvedRunnerIntent;
use d2b_core::processes::ProcessRole;

use crate::otel_host_bridge_argv::{OtelHostBridgeArgvInputs, generate_otel_host_bridge_argv};
use crate::runner_process::runner_process_metadata;
use crate::virtiofsd_argv::{VirtiofsdArgvInput, generate_virtiofsd_argv};

/// Errors that can occur during regeneration.
#[derive(Debug)]
pub enum RegenerateArgvError {
    MissingInput { role: ProcessRole, field: String },
    /// The role's generator is owned by another provider or effect boundary.
    /// The broker must keep the trusted bundle's prebuilt argv authoritative.
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
    /// Pre-built [`VirtiofsdArgvInput`] when the role is
    /// [`ProcessRole::Virtiofsd`].
    pub virtiofsd_input: Option<VirtiofsdArgvInput>,
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
/// At v1.1 this function is the documented migration surface - the
/// broker may opt into Rust-regenerated argv per-role to validate
/// against the bundle's prebuilt argv before the v1.1.1 wire
/// cleanup makes the Rust path mandatory.
pub fn regenerate_argv(
    intent: &ResolvedRunnerIntent,
    extra: &RunnerArgvExtra,
) -> Result<Vec<String>, RegenerateArgvError> {
    let metadata = runner_process_metadata(&intent.role);
    if !metadata.regenerator_wired() {
        return Err(RegenerateArgvError::NotYetWired(intent.role.clone()));
    }

    match &intent.role {
        ProcessRole::Virtiofsd => {
            let input = require(&extra.virtiofsd_input, &intent.role, "virtiofsd_input")?;
            let mut argv = generate_virtiofsd_argv(input)
                .map_err(|e| RegenerateArgvError::Generator(format!("{e:?}")))?;
            replace_arg0(&mut argv, intent);
            Ok(argv)
        }
        ProcessRole::CloudHypervisorRunner | ProcessRole::QemuMediaRunner => {
            Err(RegenerateArgvError::NotYetWired(intent.role.clone()))
        }
        ProcessRole::Swtpm
        | ProcessRole::Gpu
        | ProcessRole::GpuRenderNode
        | ProcessRole::Audio
        | ProcessRole::Video => {
            Err(RegenerateArgvError::NotYetWired(intent.role.clone()))
        }
        ProcessRole::VsockRelay => {
            Err(RegenerateArgvError::NotYetWired(intent.role.clone()))
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
        ProcessRole::Usbip => Err(RegenerateArgvError::NotYetWired(intent.role.clone())),
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::GuestSshReadiness
        | ProcessRole::GuestControlHealth
        | ProcessRole::SwtpmPreStartFlush
        | ProcessRole::SecurityKeyFrontend
        | ProcessRole::WaylandProxy => {
            unreachable!("non-wired runner role should be returned before generator dispatch")
        }
    }
}

/// Replace `argv[0]` with the resolved-intent's arg0 (`microvm@<vm>`
/// or similar). Per-role generators emit `argv[0] = binary_path` for
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
    use crate::runner_process::{RUNNER_PROCESS_MATRIX, RegeneratorWiring};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ExpectedRegeneratorClassification {
        MissingInput,
        NotYetWired,
    }

    fn expected_regenerator_classification(
        role: &ProcessRole,
    ) -> ExpectedRegeneratorClassification {
        match role {
            ProcessRole::Virtiofsd | ProcessRole::OtelHostBridge => {
                ExpectedRegeneratorClassification::MissingInput
            }
            ProcessRole::CloudHypervisorRunner | ProcessRole::QemuMediaRunner => {
                ExpectedRegeneratorClassification::NotYetWired
            }
            ProcessRole::VsockRelay => ExpectedRegeneratorClassification::NotYetWired,
            ProcessRole::HostReconcile
            | ProcessRole::StoreVirtiofsPreflight
            | ProcessRole::GuestSshReadiness
            | ProcessRole::GuestControlHealth
            | ProcessRole::SwtpmPreStartFlush
            | ProcessRole::Swtpm
            | ProcessRole::Gpu
            | ProcessRole::GpuRenderNode
            | ProcessRole::Video
            | ProcessRole::Usbip
            | ProcessRole::SecurityKeyFrontend
            | ProcessRole::Audio
            | ProcessRole::WaylandProxy => ExpectedRegeneratorClassification::NotYetWired,
        }
    }

    fn fake_intent(role: ProcessRole) -> ResolvedRunnerIntent {
        d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("fake")
            .with_vm_name("fake-vm")
            .with_role_id("fake-role")
            .with_role(role)
            .with_binary_path(std::path::PathBuf::from("/bin/fake"))
            .with_argv(vec!["fake".to_owned()])
            .with_uid(0)
            .with_gid(0)
            .with_mount_policy(d2b_core::minijail_profile::MountPolicy {
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
    fn provider_runtime_roles_remain_bundle_authoritative() {
        let intent = fake_intent(ProcessRole::CloudHypervisorRunner);
        let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
        assert!(matches!(
            err,
            RegenerateArgvError::NotYetWired(ProcessRole::CloudHypervisorRunner)
        ));
    }

    #[test]
    fn audio_role_remains_bundle_authoritative_until_runtime_wave() {
        let intent = fake_intent(ProcessRole::Audio);
        let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
        assert!(
            matches!(err, RegenerateArgvError::NotYetWired(ProcessRole::Audio)),
            "Audio regeneration must not bypass bundle-authoritative argv: {err:?}"
        );
    }

    #[test]
    fn matrix_classification_matches_pre_matrix_dispatcher() {
        for row in RUNNER_PROCESS_MATRIX {
            let intent = fake_intent(row.role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
            let actual = match err {
                RegenerateArgvError::MissingInput { .. } => {
                    ExpectedRegeneratorClassification::MissingInput
                }
                RegenerateArgvError::NotYetWired(_) => {
                    ExpectedRegeneratorClassification::NotYetWired
                }
                RegenerateArgvError::Generator(_) => {
                    panic!(
                        "generator should not run without typed inputs for {:?}",
                        row.role
                    )
                }
            };
            assert_eq!(
                actual,
                expected_regenerator_classification(&row.role),
                "classification changed for {:?}",
                row.role
            );
        }
    }

    #[test]
    fn matrix_wired_roles_keep_missing_input_classification_without_extras() {
        for row in RUNNER_PROCESS_MATRIX {
            if row.regenerator_wiring != RegeneratorWiring::Wired {
                continue;
            }

            let intent = fake_intent(row.role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
            assert!(
                matches!(err, RegenerateArgvError::MissingInput { ref role, .. } if role == &intent.role),
                "expected MissingInput for wired role {:?}, got {err:?}",
                row.role
            );
        }
    }

    #[test]
    fn matrix_not_yet_wired_roles_keep_not_yet_wired_classification() {
        for row in RUNNER_PROCESS_MATRIX {
            if row.regenerator_wiring != RegeneratorWiring::NotYetWired {
                continue;
            }

            let intent = fake_intent(row.role.clone());
            let err = regenerate_argv(&intent, &RunnerArgvExtra::default()).unwrap_err();
            assert!(
                matches!(err, RegenerateArgvError::NotYetWired(ref role) if role == &row.role),
                "expected NotYetWired for non-wired role {:?}, got {err:?}",
                row.role
            );
        }
    }

    #[test]
    fn device_provider_roles_preserve_bundle_argv_authority() {
        for role in [
            ProcessRole::Swtpm,
            ProcessRole::Gpu,
            ProcessRole::GpuRenderNode,
            ProcessRole::Video,
            ProcessRole::Usbip,
        ] {
            let intent = fake_intent(role.clone());
            assert!(matches!(
                regenerate_argv(&intent, &RunnerArgvExtra::default()),
                Err(RegenerateArgvError::NotYetWired(actual)) if actual == role
            ));
        }
    }
}
