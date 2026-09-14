//! Provider crate layout policy.
//!
//! Cargo metadata is the source of truth for workspace membership. The
//! filesystem scan is intentionally separate: a Provider-shaped crate can
//! exist under `packages/` without appearing in the workspace member list,
//! and that omission must fail closed rather than making the crate invisible
//! to this policy.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

const PROVIDER_PREFIX: &str = "d2b-provider-";
const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-config-nixos",
    "d2b-provider-supervisor",
    "d2b-provider-test-controller",
    "d2b-provider-toolkit",
];

/// The crate that declares the resource types a driver crate serves.
///
/// A per-type driver crate implements one resource type's driver, so it
/// depends on this crate; that declared dependency, and not the shape of the
/// crate name, is what separates a driver crate from a packaging Provider.
const RESOURCE_TYPES_CRATE: &str = "d2b-resource-types";

/// One row in the accepted Provider catalog.
///
/// The matrix is deliberately kept beside the workspace policy.  Cargo
/// metadata proves which crates exist, while this closed table proves that a
/// crate, dossier, owner-local test, and aggregate target describe the same
/// accepted Provider identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderMatrixRow {
    pub(crate) identity: &'static str,
    pub(crate) crate_name: &'static str,
    pub(crate) source_path: &'static str,
    pub(crate) test_path: &'static str,
    pub(crate) dossier_path: &'static str,
    pub(crate) bazel_target: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) bootstrap: bool,
}

/// The closed initial Provider matrix from the generic reconciler plan.
pub(crate) const PROVIDER_MATRIX: &[ProviderMatrixRow] = &[
    ProviderMatrixRow {
        identity: "system-core",
        crate_name: "d2b-provider-system-core",
        source_path: "packages/d2b-provider-system-core/src/host.rs",
        test_path: "packages/d2b-provider-system-core/tests/host_reconciliation.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-system-core.md",
        bazel_target: "//packages/d2b-provider-system-core:all-tests",
        unit: "U5",
        bootstrap: true,
    },
    ProviderMatrixRow {
        identity: "system-systemd",
        crate_name: "d2b-provider-process-systemd",
        source_path: "packages/d2b-provider-process-systemd/src/controller.rs",
        test_path: "packages/d2b-provider-process-systemd/tests/controller.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-system-systemd.md",
        bazel_target: "//packages/d2b-provider-process-systemd:all-tests",
        unit: "U5",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "system-minijail",
        crate_name: "d2b-provider-process-minijail",
        source_path: "packages/d2b-provider-process-minijail/src/launch.rs",
        test_path: "packages/d2b-provider-process-minijail/tests/conformance.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-system-minijail.md",
        bazel_target: "//packages/d2b-provider-process-minijail:all-tests",
        unit: "U5",
        bootstrap: true,
    },
    ProviderMatrixRow {
        identity: "runtime-cloud-hypervisor",
        crate_name: "d2b-provider-guest-cloud-hypervisor",
        source_path: "packages/d2b-provider-guest-cloud-hypervisor/src/controller.rs",
        test_path: "packages/d2b-provider-guest-cloud-hypervisor/tests/reconcile_state_machine_test.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-runtime-cloud-hypervisor.md",
        bazel_target: "//packages/d2b-provider-guest-cloud-hypervisor:all-tests",
        unit: "U6",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "runtime-qemu-media",
        crate_name: "d2b-provider-guest-qemu-media",
        source_path: "packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs",
        test_path: "packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-runtime-qemu-media.md",
        bazel_target: "//packages/d2b-provider-guest-qemu-media:all-tests",
        unit: "U6",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "runtime-azure-container-apps",
        crate_name: "d2b-provider-guest-azure-container-apps",
        source_path: "packages/d2b-provider-guest-azure-container-apps/src/controller.rs",
        test_path: "packages/d2b-provider-guest-azure-container-apps/tests/provider_lifecycle.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-runtime-azure-container-apps.md",
        bazel_target: "//packages/d2b-provider-guest-azure-container-apps:all-tests",
        unit: "U6",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "runtime-azure-virtual-machine",
        crate_name: "d2b-provider-guest-azure-virtual-machine",
        source_path: "packages/d2b-provider-guest-azure-virtual-machine/src/controller/mod.rs",
        test_path: "packages/d2b-provider-guest-azure-virtual-machine/tests/lifecycle_hermetic.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-runtime-azure-virtual-machine.md",
        bazel_target: "//packages/d2b-provider-guest-azure-virtual-machine:all-tests",
        unit: "U6",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "volume-local",
        crate_name: "d2b-provider-volume-local",
        source_path: "packages/d2b-provider-volume-local/src/controller.rs",
        test_path: "packages/d2b-provider-volume-local/tests/volume_local.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-volume-local.md",
        bazel_target: "//packages/d2b-provider-volume-local:all-tests",
        unit: "U7",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "volume-virtiofs",
        crate_name: "d2b-provider-volume-virtiofs",
        source_path: "packages/d2b-provider-volume-virtiofs/src/controller.rs",
        test_path: "packages/d2b-provider-volume-virtiofs/tests/lifecycle.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-volume-virtiofs.md",
        bazel_target: "//packages/d2b-provider-volume-virtiofs:all-tests",
        unit: "U7",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "network-local",
        crate_name: "d2b-provider-network-local",
        source_path: "packages/d2b-provider-network-local/src/controller.rs",
        test_path: "packages/d2b-provider-network-local/tests/reconcile.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-network-local.md",
        bazel_target: "//packages/d2b-provider-network-local:all-tests",
        unit: "U8",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "device-tpm",
        crate_name: "d2b-provider-device-tpm",
        source_path: "packages/d2b-provider-device-tpm/src/resource_controller.rs",
        test_path: "packages/d2b-provider-device-tpm/tests/resource_controller.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-device-tpm.md",
        bazel_target: "//packages/d2b-provider-device-tpm:all-tests",
        unit: "U8",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "device-usbip",
        crate_name: "d2b-provider-device-usbip",
        source_path: "packages/d2b-provider-device-usbip/src/controller.rs",
        test_path: "packages/d2b-provider-device-usbip/tests/service_binding_lifecycle.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-device-usbip.md",
        bazel_target: "//packages/d2b-provider-device-usbip:all-tests",
        unit: "U8",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "device-security-key",
        crate_name: "d2b-provider-device-security-key",
        source_path: "packages/d2b-provider-device-security-key/src/controller.rs",
        test_path: "packages/d2b-provider-device-security-key/tests/lease_state_machine.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-device-security-key.md",
        bazel_target: "//packages/d2b-provider-device-security-key:all-tests",
        unit: "U8",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "device-gpu",
        crate_name: "d2b-provider-device-gpu",
        source_path: "packages/d2b-provider-device-gpu/src/controller.rs",
        test_path: "packages/d2b-provider-device-gpu/tests/combined_reconcile.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-device-gpu.md",
        bazel_target: "//packages/d2b-provider-device-gpu:all-tests",
        unit: "U8",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "display-wayland",
        crate_name: "d2b-provider-display-wayland",
        source_path: "packages/d2b-provider-display-wayland/src/controller.rs",
        test_path: "packages/d2b-provider-display-wayland/tests/provider_behavior.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-display-wayland.md",
        bazel_target: "//packages/d2b-provider-display-wayland:all-tests",
        unit: "U9",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "audio-pipewire",
        crate_name: "d2b-provider-audio-pipewire",
        source_path: "packages/d2b-provider-audio-pipewire/src/controller.rs",
        test_path: "packages/d2b-provider-audio-pipewire/tests/controller.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-audio-pipewire.md",
        bazel_target: "//packages/d2b-provider-audio-pipewire:all-tests",
        unit: "U9",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "clipboard-wayland",
        crate_name: "d2b-provider-clipboard-wayland",
        source_path: "packages/d2b-provider-clipboard-wayland/src/controller/mod.rs",
        test_path: "packages/d2b-provider-clipboard-wayland/tests/provider_behavior.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-clipboard-wayland.md",
        bazel_target: "//packages/d2b-provider-clipboard-wayland:all-tests",
        unit: "U9",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "notification-desktop",
        crate_name: "d2b-provider-notification-desktop",
        source_path: "packages/d2b-provider-notification-desktop/src/controller.rs",
        test_path: "packages/d2b-provider-notification-desktop/tests/provider_behavior.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-notification-desktop.md",
        bazel_target: "//packages/d2b-provider-notification-desktop:all-tests",
        unit: "U9",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "shell-terminal",
        crate_name: "d2b-provider-shell-terminal",
        source_path: "packages/d2b-provider-shell-terminal/src/service/controller.rs",
        test_path: "packages/d2b-provider-shell-terminal/tests/controller_reconcile.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-shell-terminal.md",
        bazel_target: "//packages/d2b-provider-shell-terminal:all-tests",
        unit: "U9",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "credential-secret-service",
        crate_name: "d2b-provider-credential-secret-service",
        source_path: "packages/d2b-provider-credential-secret-service/src/controller.rs",
        test_path: "packages/d2b-provider-credential-secret-service/tests/session.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-credential-secret-service.md",
        bazel_target: "//packages/d2b-provider-credential-secret-service:all-tests",
        unit: "U10",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "credential-entra",
        crate_name: "d2b-provider-credential-entra",
        source_path: "packages/d2b-provider-credential-entra/src/controller.rs",
        test_path: "packages/d2b-provider-credential-entra/tests/controller.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-credential-entra.md",
        bazel_target: "//packages/d2b-provider-credential-entra:all-tests",
        unit: "U10",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "credential-managed-identity",
        crate_name: "d2b-provider-credential-managed-identity",
        source_path: "packages/d2b-provider-credential-managed-identity/src/controller.rs",
        test_path: "packages/d2b-provider-credential-managed-identity/tests/binding.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-credential-managed-identity.md",
        bazel_target: "//packages/d2b-provider-credential-managed-identity:all-tests",
        unit: "U10",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "transport-unix",
        crate_name: "d2b-provider-transport-unix",
        source_path: "packages/d2b-provider-transport-unix/src/portal.rs",
        test_path: "packages/d2b-provider-transport-unix/tests/transport.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-transport-unix.md",
        bazel_target: "//packages/d2b-provider-transport-unix:all-tests",
        unit: "U11",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "transport-vsock",
        crate_name: "d2b-provider-transport-vsock",
        source_path: "packages/d2b-provider-transport-vsock/src/service.rs",
        test_path: "packages/d2b-provider-transport-vsock/tests/service.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-transport-vsock.md",
        bazel_target: "//packages/d2b-provider-transport-vsock:all-tests",
        unit: "U11",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "transport-azure-relay",
        crate_name: "d2b-provider-transport-azure-relay",
        source_path: "packages/d2b-provider-transport-azure-relay/src/relay_transport.rs",
        test_path: "packages/d2b-provider-transport-azure-relay/tests/fake_relay_transport.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-transport-azure-relay.md",
        bazel_target: "//packages/d2b-provider-transport-azure-relay:all-tests",
        unit: "U11",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "observability-otel",
        crate_name: "d2b-provider-observability-otel",
        source_path: "packages/d2b-provider-observability-otel/src/controller.rs",
        test_path: "packages/d2b-provider-observability-otel/tests/binding_controller.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-observability-otel.md",
        bazel_target: "//packages/d2b-provider-observability-otel:all-tests",
        unit: "U12",
        bootstrap: false,
    },
    ProviderMatrixRow {
        identity: "activation-nixos",
        crate_name: "d2b-provider-activation-nixos",
        source_path: "packages/d2b-provider-activation-nixos/src/controller.rs",
        test_path: "packages/d2b-provider-activation-nixos/tests/reconcile.rs",
        dossier_path: "docs/specs/providers/ADR-046-provider-activation-nixos.md",
        bazel_target: "//packages/d2b-provider-activation-nixos:all-tests",
        unit: "U12",
        bootstrap: false,
    },
];
// The Provider crates whose integration surface is a recorded scaffold rather
// than an executable scenario. The set only shrinks: a listed crate that gains
// an executable scenario fails the check until its entry is deleted, no crate
// joins the set, and the entries are not exemptions from the four required
// paths or the README sections.
const README_ONLY_INTEGRATION_RATCHET: &[&str] = &[
    "d2b-provider-activation-nixos",
    "d2b-provider-audio-pipewire",
    "d2b-provider-clipboard-wayland",
    "d2b-provider-credential-entra",
    "d2b-provider-credential-managed-identity",
    "d2b-provider-credential-secret-service",
    "d2b-provider-device-gpu",
    "d2b-provider-display-wayland",
    "d2b-provider-notification-desktop",
    "d2b-provider-process-minijail",
    "d2b-provider-process-systemd",
    "d2b-provider-guest-azure-container-apps",
    "d2b-provider-guest-azure-virtual-machine",
    "d2b-provider-guest-cloud-hypervisor",
    "d2b-provider-system-core",
    "d2b-provider-transport-azure-relay",
    "d2b-provider-transport-unix",
    "d2b-provider-volume-virtiofs",
];

const REQUIRED_PATHS: &[&str] = &["src", "tests", "integration", "README.md"];
const REQUIRED_README_SECTIONS: &[&str] = &[
    "Provider identity",
    "Config schema",
    "Exported resource types",
    "Controllers / services / workers / binaries",
    "Placement and dependencies",
    "RBAC requirements",
    "Security posture",
    "State and telemetry",
    "Build and test",
];

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceMember {
    package_name: String,
    crate_dir: PathBuf,
    manifest_path: PathBuf,
    /// Whether the member manifest declares the resource-type crate, which
    /// makes a provider-prefixed member a per-type driver crate.
    declares_driver: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OnDiskProvider {
    directory_name: String,
    manifest_path: PathBuf,
    /// Whether the crate manifest declares the resource-type crate, which
    /// keeps a driver crate out of the packaging obligations and the catalog.
    declares_driver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderNameKind {
    NonProvider,
    Provider,
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Diagnostic {
    error: &'static str,
    #[serde(rename = "crate")]
    crate_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing: Option<Vec<String>>,
}

impl Diagnostic {
    fn path_missing(crate_name: &str, missing: Vec<String>) -> Self {
        Self {
            error: "missing-provider-crate-path",
            crate_name: diagnostic_name(crate_name),
            missing: Some(missing),
        }
    }

    fn readme_sections_missing(crate_name: &str, missing: Vec<String>) -> Self {
        Self {
            error: "missing-provider-readme-section",
            crate_name: diagnostic_name(crate_name),
            missing: Some(missing),
        }
    }

    fn simple(error: &'static str, crate_name: &str) -> Self {
        Self {
            error,
            crate_name: diagnostic_name(crate_name),
            missing: None,
        }
    }

    fn matrix_path(error: &'static str, crate_name: &str, path: &str) -> Self {
        Self {
            error,
            crate_name: diagnostic_name(crate_name),
            missing: Some(vec![path.to_owned()]),
        }
    }

    fn render(&self) -> String {
        serde_json::to_string(self).expect("fixed Provider policy diagnostic serializes")
    }
}

/// Check the normative layout of every Provider workspace member, ensure every
/// Provider-shaped crate on disk is represented by Cargo metadata, fail on
/// resource knowledge that still lives in a shared crate, and fail on a
/// comment citation that points at a module the tree no longer has.
pub fn check(repo_root: &Path) -> Result<(), String> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|_| "provider-crate-layout-input-unreadable".to_owned())?;
    let members = cargo_workspace_members(&repo_root)?;
    check_members(&repo_root, members.clone())?;
    check_closed_matrix(&repo_root, &members)?;
    check_shared_driver_placements(&repo_root)?;
    check_dangling_citations(&repo_root)
}

fn check_closed_matrix(
    repo_root: &Path,
    members: &[WorkspaceMember],
) -> Result<(), String> {
    let expected: BTreeSet<&str> = PROVIDER_MATRIX
        .iter()
        .map(|row| row.crate_name)
        .collect();
    let actual: BTreeSet<&str> = members
        .iter()
        .filter(|member| {
            name_kind(&member.package_name, member.declares_driver)
                == ProviderNameKind::Provider
        })
        .map(|member| member.package_name.as_str())
        .collect();
    let mut violations = Vec::new();

    for crate_name in expected.difference(&actual) {
        violations.push(Diagnostic::simple(
            "provider-matrix-row-missing",
            crate_name,
        ));
    }
    for crate_name in actual.difference(&expected) {
        violations.push(Diagnostic::simple(
            "provider-matrix-row-unexpected",
            crate_name,
        ));
    }

    for row in PROVIDER_MATRIX {
        let Some(member) = members
            .iter()
            .find(|member| member.package_name == row.crate_name)
        else {
            continue;
        };

        let dossier = repo_root.join(row.dossier_path);
        if !dossier.is_file() {
            violations.push(Diagnostic::matrix_path(
                "provider-matrix-dossier-missing",
                row.crate_name,
                row.dossier_path,
            ));
        } else {
            let expected_spec_id = format!("| Spec ID | `ADR-046-provider-{}` |", row.identity);
            let spec_id_count = fs::read_to_string(&dossier)
                .map(|text| {
                    text.lines()
                        .filter(|line| line.trim() == expected_spec_id)
                        .count()
                })
                .unwrap_or(0);
            if spec_id_count != 1 {
                violations.push(Diagnostic::simple(
                    "provider-matrix-dossier-identity-mismatch",
                    row.crate_name,
                ));
            }
        }

        let build = member.crate_dir.join("BUILD.bazel");
        let has_aggregate_target = fs::read_to_string(&build)
            .map(|text| text.contains("name = \"all-tests\""))
            .unwrap_or(false);
        if !has_aggregate_target {
            violations.push(Diagnostic::matrix_path(
                "provider-matrix-test-target-missing",
                row.crate_name,
                row.bazel_target,
            ));
        }

        // The row's own citations are part of the identity: a matrix that
        // names a deleted module validates a file list that is partly
        // fictional, so a moved module repoints the row in the same change.
        for (error, path) in [
            ("provider-matrix-source-missing", row.source_path),
            ("provider-matrix-test-missing", row.test_path),
        ] {
            if !repo_root.join(path).is_file() {
                violations.push(Diagnostic::matrix_path(error, row.crate_name, path));
            }
        }
    }

    violations.sort_by(|left, right| {
        left.crate_name
            .cmp(&right.crate_name)
            .then_with(|| left.error.cmp(right.error))
            .then_with(|| left.missing.cmp(&right.missing))
    });
    violations.dedup();

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations
            .iter()
            .map(Diagnostic::render)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// The shared-crate source roots whose modules must not declare a resource
/// driver.
///
/// A driver belongs to the per-type crate that declares its resource type.
/// These roots are the shared platform - the daemon, the broker, the core
/// contracts, the controller session library, and the resource runtime and
/// resource types the framework itself lives in - so a driver declaration here
/// is resource knowledge living outside the crate that owns it. The framework
/// roots are monitored so the shared declaration-only metadata driver is
/// policed in place: [`FRAMEWORK_DRIVER_DECLARATIONS`] names the one allowed
/// case, and a per-resource driver parked in either crate still fails.
const SHARED_CRATE_SOURCE_ROOTS: &[&str] = &[
    "packages/d2b-broker/src",
    "packages/d2b-contracts-broker/src",
    "packages/d2b-contracts-control/src",
    "packages/d2b-contracts-provider/src",
    "packages/d2b-contracts-resource/src",
    "packages/d2b-contracts-zone-session/src",
    "packages/d2b-contracts/src",
    "packages/d2b-core-controller/src",
    "packages/d2b-core/src",
    "packages/d2b-resource-runtime/src",
    "packages/d2b-resource-types/src",
    "packages/d2bd/src",
];

/// One shared-crate module that still declares a resource driver.
///
/// The list only shrinks: the change that moves a family into its own provider
/// crate deletes its entry in the same commit, a module that declares a driver
/// without an entry is a policy failure, and an entry whose module no longer
/// declares one fails the same way. The Guest move retires the last entry, so
/// no shared crate declares a driver today and the list is empty.
struct SharedDriverExemption {
    /// Repository-relative module path that declares the driver today.
    module: &'static str,
    /// The family that owns the module today.
    family: &'static str,
    /// What deletes the entry.
    retires_with: &'static str,
}

const SHARED_DRIVER_EXEMPTIONS: &[SharedDriverExemption] = &[];

/// The crate source root one shared module path belongs to.
fn shared_source_root(module: &str) -> &str {
    match module.match_indices('/').nth(1) {
        Some((index, _)) => &module[..index],
        None => module,
    }
}

/// The framework's own driver declarations, the one allowed implementation
/// shape under the framework roots.
///
/// `d2b-resource-runtime` defines the driver traits and owns the shared
/// declaration-only metadata driver every declaration-only metadata type
/// converges through. That driver and its factory are the framework's own:
/// they carry no resource-type knowledge, they serve every declaration-only
/// metadata type, and they are why the runtime and resource-type roots are
/// monitored at all. The check names them exactly instead of skipping the
/// module, so a per-resource driver parked in either crate still fails.
struct FrameworkDriverDeclaration {
    /// Repository-relative module that holds the declaration.
    module: &'static str,
    /// The type the declaration implements a driver trait for.
    implemented: &'static str,
}

const FRAMEWORK_DRIVER_DECLARATIONS: &[FrameworkDriverDeclaration] = &[
    FrameworkDriverDeclaration {
        module: "packages/d2b-resource-runtime/src/metadata.rs",
        implemented: "MetadataDriver",
    },
    FrameworkDriverDeclaration {
        module: "packages/d2b-resource-runtime/src/metadata.rs",
        implemented: "MetadataDriverFactory",
    },
];

/// Whether one declaration is the framework's own.
fn framework_driver_allowed(module: &str, implemented: &str) -> bool {
    FRAMEWORK_DRIVER_DECLARATIONS
        .iter()
        .any(|declaration| declaration.module == module && declaration.implemented == implemented)
}

/// The driver declaration one code line carries, and the type it names.
///
/// The prefix of the line is not part of the signal: a declaration can open
/// with a visibility qualifier, an attribute, or another item on the same
/// line, so the check matches the trait name and its `for` clause wherever
/// they appear. It does not match the framework's blanket
/// `impl<D: ResourceDriver> DynResourceDriver for D` glue, because
/// `DynResourceDriver` does not share the trait name's word boundary.
fn driver_declaration(code: &str) -> Option<String> {
    for trait_name in ["ResourceDriverFactory", "ResourceDriver"] {
        let mut search = code;
        while let Some(index) = search.find(trait_name) {
            let after = &search[index + trait_name.len()..];
            let boundary = search[..index]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_');
            if boundary {
                if let Some(rest) = after.strip_prefix(" for") {
                    let rest = rest.trim_start();
                    let end = rest
                        .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .unwrap_or(rest.len());
                    if end > 0 {
                        return Some(rest[..end].to_owned());
                    }
                }
            }
            search = after;
        }
    }
    None
}

/// The code half of one line: whatever precedes the first `//`.
fn code_text(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// The indent of the test-only module one line opens, when it opens one.
fn opens_test_module(lines: &[&str], index: usize) -> Option<usize> {
    if lines[index].trim() != "#[cfg(test)]" {
        return None;
    }
    for line in &lines[index + 1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("#[") || trimmed.starts_with("//") {
            continue;
        }
        let is_module = trimmed.starts_with("mod ")
            || trimmed.starts_with("pub mod ")
            || trimmed.starts_with("pub(crate) mod ")
            || trimmed.contains(" mod ");
        return is_module.then(|| line.len() - line.trim_start().len());
    }
    None
}

/// Whether one line closes a block opened at `indent`.
///
/// rustfmt keeps a block's own closing brace at the opener's indent and every
/// nested close deeper, so the first `}` at or above the opener's indent ends
/// the module - a nested `#[cfg(test)] mod` therefore stops at its own close,
/// not at the outer module's.
fn closes_indented_block(line: &str, indent: usize) -> bool {
    line.trim() == "}" && line.len() - line.trim_start().len() <= indent
}

/// The driver declarations in one module, ignoring test-only code.
///
/// Rust convention puts a file's test code behind a `#[cfg(test)]` module.
/// Test code never ships, so a driver-shaped item there is not a placement
/// decision; skipping the module keeps the check on production declarations
/// without parsing it. A declaration outside such a module is reported
/// wherever it appears, which is what makes the line prefix irrelevant.
fn driver_declarations(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut declarations = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if let Some(indent) = opens_test_module(&lines, index) {
            index += 1;
            while index < lines.len() && !closes_indented_block(lines[index], indent) {
                index += 1;
            }
            index += 1;
            continue;
        }
        let code = code_text(lines[index]);
        let code = code.split_once("/*").map_or(code, |(before, _)| before);
        if let Some(implemented) = driver_declaration(code) {
            declarations.push((index + 1, implemented));
        }
        index += 1;
    }
    declarations
}

fn collect_shared_drivers(
    repo_root: &Path,
    directory: &Path,
    declared: &mut BTreeSet<String>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        if file_type.is_dir() {
            collect_shared_drivers(repo_root, &path, declared)?;
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        let relative = path.strip_prefix(repo_root).unwrap_or(&path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        if driver_declarations(&text)
            .iter()
            .any(|(_, implemented)| !framework_driver_allowed(&relative, implemented))
        {
            declared.insert(relative);
        }
    }
    Ok(())
}

/// Every monitored module that declares a resource driver the check does not
/// allow, keyed by repository-relative path.
fn declared_shared_drivers(repo_root: &Path) -> Result<BTreeSet<String>, String> {
    let mut declared = BTreeSet::new();
    for root in SHARED_CRATE_SOURCE_ROOTS {
        let directory = repo_root.join(root);
        if !directory.is_dir() {
            continue;
        }
        collect_shared_drivers(repo_root, &directory, &mut declared)?;
    }
    Ok(declared)
}

/// Fail when a resource driver is declared outside a provider crate, and fail
/// on an exemption the tree no longer needs.
fn check_shared_driver_placements(repo_root: &Path) -> Result<(), String> {
    let declared = declared_shared_drivers(repo_root)?;
    let exempt: BTreeSet<String> = SHARED_DRIVER_EXEMPTIONS
        .iter()
        .map(|exemption| exemption.module.to_owned())
        .collect();
    let mut violations = Vec::new();

    for module in declared.difference(&exempt) {
        violations.push(render_shared_driver_violation(
            "shared-crate-driver-placement",
            module,
            None,
        ));
    }
    for exemption in SHARED_DRIVER_EXEMPTIONS {
        if !repo_root.join(shared_source_root(exemption.module)).is_dir() {
            continue;
        }
        if !declared.contains(exemption.module) {
            violations.push(render_shared_driver_violation(
                "stale-shared-driver-exemption",
                exemption.module,
                Some(exemption.family),
            ));
        }
    }
    for declaration in FRAMEWORK_DRIVER_DECLARATIONS {
        if !repo_root.join(shared_source_root(declaration.module)).is_dir() {
            continue;
        }
        let present = fs::read_to_string(repo_root.join(declaration.module))
            .map(|text| {
                driver_declarations(&text)
                    .iter()
                    .any(|(_, implemented)| implemented == declaration.implemented)
            })
            .unwrap_or(false);
        if !present {
            violations.push(render_shared_driver_violation(
                "stale-framework-driver-declaration",
                declaration.module,
                None,
            ));
        }
    }

    violations.sort();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations.join("\n"))
    }
}

/// Render one shared-driver placement diagnostic as canonical JSON.
fn render_shared_driver_violation(error: &str, module: &str, family: Option<&str>) -> String {
    let mut diagnostic = serde_json::json!({
        "error": error,
        "module": module,
    });
    if let Some(family) = family {
        diagnostic["family"] = serde_json::Value::String(family.to_owned());
        diagnostic["retiresWith"] = serde_json::Value::String(
            SHARED_DRIVER_EXEMPTIONS
                .iter()
                .find(|exemption| exemption.module == module)
                .map(|exemption| exemption.retires_with.to_owned())
                .unwrap_or_default(),
        );
    }
    diagnostic.to_string()
}

/// One comment citation in a monitored root whose target no longer exists.
///
/// A citation is a repository path or a crate-qualified module path as a
/// comment writes it: `packages/<crate>/src/...`, `crate::<module>`, or
/// `<crate>::<module>`. The check fails on one that does not resolve, because
/// after a module move the citation that remains is a live pointer to a file
/// that no longer exists - the class readers kept finding by hand while no
/// gate looked at comments.
#[derive(Clone)]
struct DanglingCitation {
    /// Repository-relative module that holds the comment.
    module: String,
    /// One-based line number inside that module.
    line: usize,
    /// The citation exactly as the comment writes it.
    token: String,
    /// The comment line, trimmed, for the report.
    comment: String,
    /// How the fix pass can repair the line, when the repair is mechanical.
    rewrite: Option<CitationRewrite>,
}

/// A mechanical repair of one dangling citation.
#[derive(Clone, Copy)]
enum CitationRewrite {
    /// The comment line holds only the citation: delete the line.
    DropLine,
    /// Delete `start..end` (line byte offsets), keeping `keep` in the gap.
    DropSpan {
        start: usize,
        end: usize,
        keep: Option<char>,
    },
}

/// Fail on every comment citation under the monitored roots that no longer
/// resolves.
fn check_dangling_citations(repo_root: &Path) -> Result<(), String> {
    let citations = dangling_citations(repo_root)?;
    if citations.is_empty() {
        return Ok(());
    }
    Err(citations
        .iter()
        .map(render_citation_diagnostic)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Remove every mechanically removable dangling citation, leaving the rest.
///
/// This is the `--fix` half of the check: it edits comments only, never code,
/// and it refuses a citation woven into a sentence, so the human fix states
/// the behavior instead of citing the missing file. Check mode verifies every
/// rewrite afterwards, and a second run over a fixed tree changes nothing.
pub fn fix(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|_| "provider-crate-layout-input-unreadable".to_owned())?;
    let mut by_module: BTreeMap<String, Vec<DanglingCitation>> = BTreeMap::new();
    for citation in dangling_citations(&repo_root)? {
        by_module
            .entry(citation.module.clone())
            .or_default()
            .push(citation);
    }
    let mut written = Vec::new();
    let mut remaining = Vec::new();
    for (module, citations) in by_module {
        let path = repo_root.join(&module);
        let text = fs::read_to_string(&path)
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        remaining.extend(apply_citation_fixes(&mut lines, &citations));
        let mut fixed = lines.join("\n");
        if text.ends_with('\n') {
            fixed.push('\n');
        }
        if fixed != text {
            fs::write(&path, &fixed)
                .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
            written.push(path);
        }
    }
    written.sort();
    if remaining.is_empty() {
        return Ok(written);
    }
    Err(format!(
        "{} dangling comment citation(s) need a human - state the behavior instead of citing the missing target:\n{}",
        remaining.len(),
        remaining
            .iter()
            .map(render_citation_diagnostic)
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

/// Apply the mechanical rewrites in one module, returning the citations that
/// still need a human.
fn apply_citation_fixes(
    lines: &mut Vec<String>,
    citations: &[DanglingCitation],
) -> Vec<DanglingCitation> {
    let mut by_line: BTreeMap<usize, Vec<&DanglingCitation>> = BTreeMap::new();
    for citation in citations {
        by_line.entry(citation.line).or_default().push(citation);
    }
    let mut remaining = Vec::new();
    let mut drop_lines = Vec::new();
    for (line_number, on_line) in by_line {
        let index = line_number - 1;
        if index >= lines.len() {
            remaining.extend(on_line.into_iter().cloned());
            continue;
        }
        if on_line.len() == 1 && matches!(on_line[0].rewrite, Some(CitationRewrite::DropLine)) {
            drop_lines.push(index);
            continue;
        }
        let mut spans = Vec::new();
        for citation in &on_line {
            match citation.rewrite {
                Some(CitationRewrite::DropSpan { start, end, keep }) => {
                    spans.push((start, end, keep));
                }
                _ => remaining.push((*citation).clone()),
            }
        }
        spans.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        let mut line = lines[index].clone();
        for (start, end, keep) in spans {
            let replacement: String = keep.map(String::from).unwrap_or_default();
            line.replace_range(start..end, &replacement);
        }
        lines[index] = tidy_comment(&line);
    }
    for index in drop_lines.into_iter().rev() {
        lines.remove(index);
    }
    remaining.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.token.cmp(&right.token))
    });
    remaining
}

/// Collapse the whitespace and orphaned punctuation a removal can leave.
fn tidy_comment(line: &str) -> String {
    let Some(comment_start) = comment_start(line) else {
        return line.to_owned();
    };
    let (code, comment) = line.split_at(comment_start);
    let mut comment = comment.to_owned();
    while comment.contains("  ") {
        comment = comment.replace("  ", " ");
    }
    for (orphan, tidy) in [(" ,", ","), (" .", "."), (" ;", ";"), (" )", ")")] {
        comment = comment.replace(orphan, tidy);
    }
    format!("{code}{}", comment.trim_end())
}

/// Render one dangling citation as canonical JSON.
fn render_citation_diagnostic(citation: &DanglingCitation) -> String {
    serde_json::json!({
        "error": "dangling-comment-citation",
        "module": &citation.module,
        "line": citation.line,
        "citation": &citation.token,
        "comment": &citation.comment,
    })
    .to_string()
}

/// Every dangling comment citation in the monitored roots.
fn dangling_citations(repo_root: &Path) -> Result<Vec<DanglingCitation>, String> {
    let crates = workspace_crates(repo_root)?;
    let mut roots = BTreeMap::new();
    let mut citations = Vec::new();
    for root in SHARED_CRATE_SOURCE_ROOTS {
        let directory = repo_root.join(root);
        if !directory.is_dir() {
            continue;
        }
        collect_dangling_citations(repo_root, &directory, &crates, &mut roots, &mut citations)?;
    }
    citations.sort_by(|left, right| {
        left.module
            .cmp(&right.module)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.token.cmp(&right.token))
    });
    Ok(citations)
}

fn collect_dangling_citations(
    repo_root: &Path,
    directory: &Path,
    crates: &BTreeMap<String, String>,
    roots: &mut BTreeMap<String, String>,
    citations: &mut Vec<DanglingCitation>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        if file_type.is_dir() {
            collect_dangling_citations(repo_root, &path, crates, roots, citations)?;
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        let relative = path.strip_prefix(repo_root).unwrap_or(&path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        let own_crate = own_crate_of(&relative);
        let lines: Vec<&str> = text.lines().collect();
        for (offset, line) in lines.iter().enumerate() {
            let Some(comment_start) = comment_start(line) else {
                continue;
            };
            let previous = offset.checked_sub(1).map(|index| lines[index]);
            for token in dangling_comment_citations(
                repo_root,
                crates,
                roots,
                own_crate.as_deref(),
                line,
                comment_start,
                previous,
            )? {
                citations.push(DanglingCitation {
                    module: relative.clone(),
                    line: offset + 1,
                    token: token.text,
                    comment: line.trim().to_owned(),
                    rewrite: token.rewrite,
                });
            }
        }
    }
    Ok(())
}

/// One dangling citation found inside a comment.
struct CommentCitation {
    text: String,
    rewrite: Option<CitationRewrite>,
}

/// Every citation in one comment line whose target no longer resolves.
#[allow(clippy::too_many_arguments)]
fn dangling_comment_citations(
    repo_root: &Path,
    crates: &BTreeMap<String, String>,
    roots: &mut BTreeMap<String, String>,
    own_crate: Option<&str>,
    line: &str,
    comment_start: usize,
    previous: Option<&str>,
) -> Result<Vec<CommentCitation>, String> {
    let comment = &line[comment_start..];
    let mut citations = Vec::new();
    for (start, end) in path_token_spans(comment) {
        let raw = &comment[start..end];
        if resolve_path_citation(repo_root, raw) {
            continue;
        }
        citations.push(comment_citation(
            line,
            comment_start,
            previous,
            start,
            end,
            raw,
        ));
    }
    for (start, end, qualifier, segment) in module_token_spans(comment) {
        let raw = &comment[start..end];
        if resolve_module_citation(repo_root, crates, roots, own_crate, &qualifier, &segment)? {
            continue;
        }
        citations.push(comment_citation(
            line,
            comment_start,
            previous,
            start,
            end,
            raw,
        ));
    }
    Ok(citations)
}

fn comment_citation(
    line: &str,
    comment_start: usize,
    previous: Option<&str>,
    start: usize,
    end: usize,
    token: &str,
) -> CommentCitation {
    CommentCitation {
        text: token.to_owned(),
        rewrite: mechanical_rewrite(line, comment_start, previous, start, end),
    }
}

/// The byte offset where one line's comment starts, ignoring `//` inside a
/// string literal.
fn comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            return Some(index);
        }
        index += 1;
    }
    None
}

/// The `packages/...` runs in one comment, as comment-relative byte spans.
fn path_token_spans(comment: &str) -> Vec<(usize, usize)> {
    let bytes = comment.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while let Some(offset) = comment[index..].find("packages/") {
        let start = index + offset;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric()
                || matches!(bytes[end], b'/' | b'_' | b'-' | b'.'))
        {
            end += 1;
        }
        spans.push((start, end));
        index = end.max(start + 1);
    }
    spans
}

/// Whether one `packages/...` citation names something on disk.
fn resolve_path_citation(repo_root: &Path, raw: &str) -> bool {
    let Some(candidate) = path_citation_candidate(raw) else {
        return true;
    };
    repo_root.join(candidate).exists()
}

/// The path part of one citation, without a `::item` suffix or trailing
/// sentence punctuation.
fn path_citation_candidate(raw: &str) -> Option<&str> {
    let mut candidate = raw;
    if let Some(index) = candidate.find(".rs::") {
        candidate = &candidate[..index + 3];
    }
    candidate = candidate.trim_end_matches(|ch: char| {
        matches!(ch, '.' | ',' | ';' | ')' | ']' | '}' | '`' | '\'' | '"')
    });
    if candidate.contains('*') || candidate.len() <= "packages/".len() {
        return None;
    }
    Some(candidate)
}

/// The crate-qualified module chains in one comment: byte spans with their
/// qualifier and first segment.
fn module_token_spans(comment: &str) -> Vec<(usize, usize, String, String)> {
    let bytes = comment.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !byte.is_ascii_alphabetic() && byte != b'_' {
            index += 1;
            continue;
        }
        let (first, first_end) = identifier_run(comment, index);
        if !comment[first_end..].starts_with("::") {
            index = first_end;
            continue;
        }
        let mut chain = vec![first];
        let mut cursor = first_end;
        while comment[cursor..].starts_with("::") {
            let rest = &comment[cursor + 2..];
            if !rest
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            {
                break;
            }
            let (segment, end) = identifier_run(comment, cursor + 2);
            chain.push(segment);
            cursor = end;
        }
        if chain.len() < 2 {
            index = cursor.max(index + 1);
            continue;
        }
        spans.push((index, cursor, chain[0].to_owned(), chain[1].to_owned()));
        index = cursor;
    }
    spans
}

/// One identifier-shaped run: `[A-Za-z_][A-Za-z0-9_-]*`.
fn identifier_run(text: &str, start: usize) -> (&str, usize) {
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
    {
        end += 1;
    }
    (&text[start..end], end)
}

/// Whether one crate-qualified module citation names a module, item, or
/// re-export of a workspace crate.
///
/// Only the first segment after the qualifier is resolved: a citation whose
/// module still exists stays valid even when a deeper path moved, and a
/// qualifier that is not a workspace crate is not this check's to resolve.
fn resolve_module_citation(
    repo_root: &Path,
    crates: &BTreeMap<String, String>,
    roots: &mut BTreeMap<String, String>,
    own_crate: Option<&str>,
    qualifier: &str,
    segment: &str,
) -> Result<bool, String> {
    let crate_dir = if qualifier == "crate" {
        match own_crate {
            Some(crate_dir) => crate_dir.to_owned(),
            None => return Ok(true),
        }
    } else {
        match crates.get(&qualifier.replace('-', "_")) {
            Some(crate_dir) => crate_dir.clone(),
            None => return Ok(true),
        }
    };
    let src = repo_root.join("packages").join(&crate_dir).join("src");
    if src.join(format!("{segment}.rs")).is_file() || src.join(segment).join("mod.rs").is_file() {
        return Ok(true);
    }
    if !roots.contains_key(&crate_dir) {
        roots.insert(crate_dir.clone(), crate_root_text(repo_root, &crate_dir)?);
    }
    Ok(root_binds_name(
        roots.get(&crate_dir).map(String::as_str).unwrap_or_default(),
        segment,
    ))
}

/// The crate a monitored module belongs to, when `crate::` citations can be
/// resolved against it.
fn own_crate_of(module: &str) -> Option<String> {
    let mut segments = module.split('/');
    if segments.next() != Some("packages") {
        return None;
    }
    let crate_dir = segments.next()?.to_owned();
    if module.contains("/src/bin/") {
        return None;
    }
    Some(crate_dir)
}

/// The workspace crates whose underscore names can qualify a comment
/// citation.
fn workspace_crates(repo_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let entries = fs::read_dir(repo_root.join("packages"))
        .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
    let mut crates = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        if !entry.path().join("src").is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        crates.insert(name.replace('-', "_"), name);
    }
    Ok(crates)
}

/// The crate-root source text: `lib.rs`, `main.rs`, and the files they
/// `include!`, which is where a root-level module, item, or re-export binding
/// can appear.
fn crate_root_text(repo_root: &Path, crate_dir: &str) -> Result<String, String> {
    let src = repo_root.join("packages").join(crate_dir).join("src");
    let mut text = String::new();
    for root in ["lib.rs", "main.rs"] {
        let path = src.join(root);
        if !path.is_file() {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?;
        for included in include_targets(&contents) {
            let included = src.join(included);
            if included.is_file() {
                text.push_str(
                    &fs::read_to_string(&included)
                        .map_err(|_| "provider-crate-layout-shared-unreadable".to_owned())?,
                );
                text.push('\n');
            }
        }
        text.push_str(&contents);
        text.push('\n');
    }
    Ok(text)
}

/// The `include!("...")` targets in one source text.
fn include_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut search = text;
    while let Some(index) = search.find("include!(\"") {
        let rest = &search[index + "include!(\"".len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        targets.push(rest[..end].to_owned());
        search = &rest[end..];
    }
    targets
}

/// Whether crate-root text binds `name` as a module, item, or import.
fn root_binds_name(text: &str, name: &str) -> bool {
    for line in text.lines() {
        if item_binding(code_text(line), name) {
            return true;
        }
    }
    use_statements(text)
        .iter()
        .any(|statement| use_binding(statement, name))
}

/// Whether one declaration line binds `name` through an item keyword.
fn item_binding(line: &str, name: &str) -> bool {
    for keyword in [
        "mod", "struct", "enum", "trait", "fn", "const", "static", "type", "union",
    ] {
        let mut search = line;
        while let Some(index) = search.find(keyword) {
            let after = &search[index + keyword.len()..];
            let boundary = search[..index]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_');
            if boundary && starts_with_whitespace(after) {
                if let Some(binding) = after.trim_start().strip_prefix(name) {
                    if ends_identifier(binding) {
                        return true;
                    }
                }
            }
            search = after;
        }
    }
    if let Some(index) = line.find("macro_rules!") {
        if let Some(binding) = line[index + "macro_rules!".len()..]
            .trim_start()
            .strip_prefix(name)
        {
            if ends_identifier(binding) {
                return true;
            }
        }
    }
    false
}

fn starts_with_whitespace(text: &str) -> bool {
    text.chars().next().is_some_and(char::is_whitespace)
}

/// Whether the text after a candidate binding ends the identifier.
fn ends_identifier(text: &str) -> bool {
    text.chars()
        .next()
        .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
}

/// Every `use` statement in one source text, whitespace-collapsed so a
/// multi-line statement matches on one string.
fn use_statements(text: &str) -> Vec<String> {
    let code = text
        .lines()
        .map(code_text)
        .collect::<Vec<_>>()
        .join(" ");
    let mut statements = Vec::new();
    let mut index = 0;
    while let Some(offset) = code[index..].find("use ") {
        let start = index + offset;
        let Some(end) = code[start..].find(';') else {
            break;
        };
        statements.push(
            code[start..start + end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
        index = start + end + 1;
    }
    statements
}

/// Whether one `use` statement binds `name`.
fn use_binding(statement: &str, name: &str) -> bool {
    if let Some((_, group)) = statement.split_once('{') {
        let group = group.split('}').next().unwrap_or_default();
        return group.split(',').any(|entry| {
            entry
                .trim()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                == name
        });
    }
    statement
        .rsplit("::")
        .next()
        .is_some_and(|last| last.trim() == name)
}

/// The mechanical repair for one dangling citation, when removing it cannot
/// change what the sentence says.
///
/// Three shapes qualify: a parenthetical that holds only the citation, a
/// trailing clause after a comma or dash that is only the citation, and a
/// comment line that holds only the citation. Everything else - a citation
/// woven into a sentence, a doc-link definition, a wrapped continuation - is
/// left for a human who can restate the behavior.
fn mechanical_rewrite(
    line: &str,
    comment_start: usize,
    previous: Option<&str>,
    start: usize,
    end: usize,
) -> Option<CitationRewrite> {
    let comment = &line[comment_start..];
    let token_start = start - comment_start;
    let token_end = end - comment_start;
    let token = &comment[token_start..token_end];
    whole_line_rewrite(comment, previous, token)
        .or_else(|| parenthetical_rewrite(comment, comment_start, token_start, token_end))
        .or_else(|| trailing_clause_rewrite(comment, comment_start, token_start, token_end, token))
}

/// A comment line that holds only the citation can go as a whole, unless the
/// previous comment line continues into it.
fn whole_line_rewrite(
    comment: &str,
    previous: Option<&str>,
    token: &str,
) -> Option<CitationRewrite> {
    let body = comment.trim_start_matches('/').trim();
    let bare = body
        .trim_matches('`')
        .trim_end_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':'))
        .trim();
    if bare != token {
        return None;
    }
    if let Some(previous) = previous {
        let previous = previous.trim();
        if previous.starts_with("//") {
            let previous_body = previous.trim_start_matches('/').trim();
            let continues = !previous_body.is_empty()
                && previous_body
                    .chars()
                    .last()
                    .is_some_and(|ch| !matches!(ch, '.' | ':' | ';' | '!' | '?'));
            if continues {
                return None;
            }
        }
    }
    Some(CitationRewrite::DropLine)
}

/// A parenthetical that holds only the citation can go with it.
fn parenthetical_rewrite(
    comment: &str,
    comment_start: usize,
    token_start: usize,
    token_end: usize,
) -> Option<CitationRewrite> {
    let open = comment[..token_start].rfind('(')?;
    let before = comment[open + 1..token_start]
        .trim()
        .trim_end_matches('`')
        .trim();
    if !before.is_empty() && !is_citation_cue(before) {
        return None;
    }
    let close = comment[token_end..].find(')')?;
    let after = comment[token_end..token_end + close]
        .trim()
        .trim_start_matches('`')
        .trim();
    if !after.is_empty() {
        return None;
    }
    Some(CitationRewrite::DropSpan {
        start: comment_start + open,
        end: comment_start + token_end + close + 1,
        keep: None,
    })
}

/// A trailing clause after a comma or dash that holds only the citation can
/// go, keeping the sentence's final punctuation.
fn trailing_clause_rewrite(
    comment: &str,
    comment_start: usize,
    token_start: usize,
    token_end: usize,
    token: &str,
) -> Option<CitationRewrite> {
    let before = comment[..token_start].trim_end();
    let before = before.strip_suffix('`').map_or(before, str::trim_end);
    let separator_index = before.rfind(|ch: char| matches!(ch, ',' | '-'))?;
    let separator = comment[separator_index..].chars().next()?;
    let clause = comment[separator_index + separator.len_utf8()..token_end].replace('`', "");
    let clause = clause.trim();
    let clause_matches = clause == token
        || clause
            .split_once(' ')
            .is_some_and(|(cue, cited)| is_citation_cue(cue) && cited.trim() == token);
    if !clause_matches {
        return None;
    }
    let after = &comment[token_end..];
    let mut cursor = token_end;
    if after.starts_with('`') {
        cursor += 1;
    }
    let keep = comment[cursor..]
        .chars()
        .next()
        .filter(|ch| matches!(ch, '.' | ';' | ','));
    if let Some(ch) = keep {
        cursor += ch.len_utf8();
    }
    if !comment[cursor..].trim().is_empty() {
        return None;
    }
    Some(CitationRewrite::DropSpan {
        start: comment_start + separator_index,
        end: comment_start + cursor,
        keep,
    })
}

fn is_citation_cue(text: &str) -> bool {
    matches!(text, "see" | "cf" | "in" | "from" | "under" | "at" | "per")
}

fn check_members(repo_root: &Path, members: Vec<WorkspaceMember>) -> Result<(), String> {
    let on_disk = on_disk_providers(&repo_root)?;
    let has_provider_member = members.iter().any(|member| {
        name_kind(&member.package_name, member.declares_driver) == ProviderNameKind::Provider
    });
    if !has_provider_member && on_disk.is_empty() {
        return Err("provider-crate-layout-empty-scope".to_owned());
    }

    let member_by_manifest: BTreeMap<PathBuf, &WorkspaceMember> = members
        .iter()
        .map(|member| (member.manifest_path.clone(), member))
        .collect();
    let mut violations = Vec::new();

    for member in &members {
        match name_kind(&member.package_name, member.declares_driver) {
            ProviderNameKind::Provider => {
                if !is_provider_directory(&repo_root, &member.crate_dir, &member.package_name) {
                    violations.push(Diagnostic::simple(
                        "provider-crate-location-invalid",
                        &member.package_name,
                    ));
                } else {
                    violations.extend(inspect_crate(member)?);
                }
            }
            ProviderNameKind::Malformed => violations.push(Diagnostic::simple(
                "provider-crate-name-invalid",
                &member.package_name,
            )),
            ProviderNameKind::NonProvider => {}
        }
    }

    for crate_on_disk in on_disk {
        match member_by_manifest.get(&crate_on_disk.manifest_path) {
            None => violations.push(Diagnostic::simple(
                "provider-crate-not-workspace-member",
                &crate_on_disk.directory_name,
            )),
            Some(member) => {
                if member.package_name != crate_on_disk.directory_name {
                    violations.push(Diagnostic::simple(
                        "provider-crate-name-mismatch",
                        &crate_on_disk.directory_name,
                    ));
                }
                if name_kind(&crate_on_disk.directory_name, crate_on_disk.declares_driver)
                    == ProviderNameKind::Malformed
                {
                    violations.push(Diagnostic::simple(
                        "provider-crate-name-invalid",
                        &crate_on_disk.directory_name,
                    ));
                }
            }
        }
    }

    violations.sort_by(|left, right| {
        left.crate_name
            .cmp(&right.crate_name)
            .then_with(|| left.error.cmp(right.error))
            .then_with(|| left.missing.cmp(&right.missing))
    });
    violations.dedup();

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations
            .iter()
            .map(Diagnostic::render)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn cargo_workspace_members(repo_root: &Path) -> Result<Vec<WorkspaceMember>, String> {
    let metadata = cargo_metadata(repo_root)?;
    let packages_by_id: BTreeMap<&str, &CargoPackage> = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect();

    let mut members = Vec::new();
    for member_id in metadata.workspace_members {
        let package = packages_by_id
            .get(member_id.as_str())
            .ok_or_else(|| "provider-crate-layout-metadata-member-missing".to_owned())?;
        let manifest_path = package
            .manifest_path
            .canonicalize()
            .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?;
        let crate_dir = manifest_path
            .parent()
            .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
            .to_owned();
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?;
        members.push(WorkspaceMember {
            package_name: package.name.clone(),
            crate_dir,
            manifest_path,
            declares_driver: manifest_declares_driver(&manifest),
        });
    }
    if members.is_empty() {
        return Err("provider-crate-layout-members-empty".to_owned());
    }
    Ok(members)
}

fn cargo_metadata(repo_root: &Path) -> Result<CargoMetadata, String> {
    let cargo = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_relative() {
                std::env::current_dir()
                    .map(|current| current.join(path))
                    .unwrap_or_else(|_| PathBuf::from("cargo"))
            } else {
                path
            }
        })
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let mut command = Command::new(cargo);
    if let Some(tmpdir) = std::env::var_os("TEST_TMPDIR") {
        let cargo_home = PathBuf::from(tmpdir).join("cargo-home");
        fs::create_dir_all(&cargo_home)
            .map_err(|_| "provider-crate-layout-metadata-home-unavailable".to_owned())?;
        command.env("CARGO_HOME", cargo_home);
    }
    let output = command
        .current_dir(repo_root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(repo_root.join("Cargo.toml"))
        .output()
        .map_err(|_| "provider-crate-layout-metadata-unavailable".to_owned())?;
    if !output.status.success() {
        return Err("provider-crate-layout-metadata-failed".to_owned());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "provider-crate-layout-metadata-malformed".to_owned())
}

fn on_disk_providers(repo_root: &Path) -> Result<Vec<OnDiskProvider>, String> {
    let packages_dir = repo_root.join("packages");
    let entries = fs::read_dir(&packages_dir)
        .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
    let mut providers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        if !file_type.is_dir() {
            continue;
        }
        let directory_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
            .to_owned();
        if !directory_name.starts_with(PROVIDER_PREFIX) {
            continue;
        }
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        let declares_driver = manifest_declares_driver(&manifest);
        if matches!(
            name_kind(&directory_name, declares_driver),
            ProviderNameKind::NonProvider
        ) {
            continue;
        }
        providers.push(OnDiskProvider {
            directory_name,
            manifest_path: manifest_path
                .canonicalize()
                .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?,
            declares_driver,
        });
    }
    providers.sort_by(|left, right| left.directory_name.cmp(&right.directory_name));
    Ok(providers)
}

/// Whether a package manifest declares a dependency on the resource types.
///
/// The declaration is read from the manifest text because the workspace
/// metadata the check already uses does not carry per-member dependencies. A
/// dependency counts in either spelling Cargo allows: the crate name as the
/// key, or the crate name as the package of a renamed key.
fn manifest_declares_driver(manifest: &str) -> bool {
    manifest.lines().any(|line| {
        let line = line.split('#').next().unwrap_or_default().trim();
        if let Some(table) = line
            .strip_prefix('[')
            .and_then(|table| table.strip_suffix(']'))
        {
            return table
                .rsplit('.')
                .next()
                .is_some_and(|key| key.trim_matches(['"', '\'']) == RESOURCE_TYPES_CRATE);
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        key.trim().split('.').next().unwrap_or_default() == RESOURCE_TYPES_CRATE
            || value
                .trim()
                .contains(&format!("package = \"{RESOURCE_TYPES_CRATE}\""))
    })
}

/// Classify one provider-prefixed crate name for this policy.
///
/// The classification decides packaging obligations and the catalog row, so
/// the name alone cannot settle it: a per-type driver crate is named after the
/// resource type it serves, and that type name may contain a dash, which is
/// exactly the shape of a packaging Provider identity.
fn provider_name_kind(name: &str, declares_driver: bool) -> ProviderNameKind {
    if NON_PROVIDER_PREFIXED.contains(&name) {
        return ProviderNameKind::NonProvider;
    }
    let Some(rest) = name.strip_prefix(PROVIDER_PREFIX) else {
        return ProviderNameKind::NonProvider;
    };
    let segments: Vec<_> = rest.split('-').collect();
    if segments.iter().any(|segment| !valid_name_segment(segment)) {
        return ProviderNameKind::Malformed;
    }
    // A driver crate declares one resource type's driver for the plane and
    // ships no packaging artifact of its own, so it carries neither the
    // packaging obligations nor a catalog row. A single segment names the same
    // kind of crate without declaring the shared resource types.
    if declares_driver || segments.len() < 2 {
        return ProviderNameKind::NonProvider;
    }
    ProviderNameKind::Provider
}

/// Whether the closed matrix names this crate as an accepted Provider.
fn catalogued_provider(crate_name: &str) -> bool {
    PROVIDER_MATRIX
        .iter()
        .any(|row| row.crate_name == crate_name)
}

/// The packaging kind of one provider-prefixed crate.
///
/// The closed matrix is the authority for the crates it names. A realizer that
/// hosts the driver of the types it realizes keeps its packaging identity, its
/// catalog row, and the artifact the Nix layer compiles for it; the declared
/// driver dependency separates the per-type driver crates the matrix does not
/// name, which ship no packaging artifact of their own.
fn name_kind(crate_name: &str, declares_driver: bool) -> ProviderNameKind {
    if catalogued_provider(crate_name) {
        return ProviderNameKind::Provider;
    }
    provider_name_kind(crate_name, declares_driver)
}

fn valid_name_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn diagnostic_name(name: &str) -> String {
    if name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        name.to_owned()
    } else {
        "<invalid-provider-crate>".to_owned()
    }
}

fn is_provider_directory(repo_root: &Path, crate_dir: &Path, package_name: &str) -> bool {
    let Some(packages_dir) = repo_root.join("packages").canonicalize().ok() else {
        return false;
    };
    crate_dir.parent() == Some(packages_dir.as_path())
        && crate_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == package_name)
}

fn inspect_crate(member: &WorkspaceMember) -> Result<Vec<Diagnostic>, String> {
    let crate_name = &member.package_name;
    let mut violations = Vec::new();
    let mut missing = Vec::new();

    for required in REQUIRED_PATHS {
        let path = member.crate_dir.join(required);
        let present = if *required == "README.md" {
            path.is_file()
        } else {
            path.is_dir()
        };
        if !present {
            missing.push((*required).to_owned());
        }
    }
    if member.crate_dir.join("src").is_dir() && !contains_rust_file(&member.crate_dir.join("src"))?
    {
        missing.push("src/*.rs".to_owned());
    }
    if member.crate_dir.join("tests").is_dir()
        && !contains_rust_file(&member.crate_dir.join("tests"))?
    {
        missing.push("tests/*.rs".to_owned());
    }

    let integration = member.crate_dir.join("integration");
    if integration.is_dir() {
        if !integration.join("README.md").is_file() {
            missing.push("integration/README.md".to_owned());
        }
        let has_rust_scenario = integration_has_rust_scenario(&integration)?;
        if !has_rust_scenario && !README_ONLY_INTEGRATION_RATCHET.contains(&crate_name.as_str()) {
            missing.push("integration/*.rs".to_owned());
        }
        if has_rust_scenario && README_ONLY_INTEGRATION_RATCHET.contains(&crate_name.as_str()) {
            return Err("provider-crate-layout-stale-exemption".to_owned());
        }
    }

    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        violations.push(Diagnostic::path_missing(crate_name, missing));
    }

    let readme = member.crate_dir.join("README.md");
    if readme.is_file() {
        let text = fs::read_to_string(&readme)
            .map_err(|_| "provider-crate-layout-readme-unreadable".to_owned())?;
        let present: BTreeSet<String> = text.lines().filter_map(heading_text).collect();
        let missing_sections = REQUIRED_README_SECTIONS
            .iter()
            .filter(|section| !present.contains(&section.to_lowercase()))
            .map(|section| format!("README.md section: {section}"))
            .collect::<Vec<_>>();
        if !missing_sections.is_empty() {
            violations.push(Diagnostic::readme_sections_missing(
                crate_name,
                missing_sections,
            ));
        }
    }

    Ok(violations)
}

fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let stripped = trimmed.strip_prefix('#')?;
    Some(stripped.trim_start_matches('#').trim().to_lowercase())
}

fn contains_rust_file(root: &Path) -> Result<bool, String> {
    let entries =
        fs::read_dir(root).map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
        if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        {
            return Ok(true);
        }
        if file_type.is_dir() && contains_rust_file(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn integration_has_rust_scenario(integration: &Path) -> Result<bool, String> {
    let entries = fs::read_dir(integration)
        .map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
        if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::repo_root;

    use super::*;

    static FIXTURE_COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let serial = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "d2b-provider-layout-{}-{serial}-{label}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            write_package(&root, "d2b-core");
            write_package(&root, "d2b-provider-fixture-example");
            let provider = root.join("packages/d2b-provider-fixture-example");
            fs::create_dir_all(provider.join("integration")).unwrap();
            fs::create_dir_all(provider.join("tests")).unwrap();
            fs::write(
                provider.join("tests/scenario.rs"),
                "#[test]\nfn fixture() {}\n",
            )
            .unwrap();
            fs::write(
                provider.join("integration/README.md"),
                "# integration fixtures\n",
            )
            .unwrap();
            fs::write(
                provider.join("integration/scenario.rs"),
                "//! integration-target: container\n",
            )
            .unwrap();
            fs::write(
                provider.join("README.md"),
                required_readme("fixture-example"),
            )
            .unwrap();
            fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers = [\n    \"packages/d2b-core\",\n    \"packages/d2b-provider-fixture-example\",\n]\n",
            )
            .unwrap();
            Self { root }
        }

        fn provider_dir(&self) -> PathBuf {
            self.root.join("packages/d2b-provider-fixture-example")
        }

        fn add_package(&self, name: &str) -> PathBuf {
            write_package(&self.root, name);
            self.root.join("packages").join(name)
        }

        fn set_members(&self, members: &[&str]) {
            let mut manifest = String::from("[workspace]\nmembers = [\n");
            for member in members {
                manifest.push_str(&format!("    \"packages/{member}\",\n"));
            }
            manifest.push_str("]\n");
            fs::write(self.root.join("Cargo.toml"), manifest).unwrap();
        }
    }

    fn manifest_workspace_members(root: &Path) -> Result<Vec<WorkspaceMember>, String> {
        let workspace = fs::read_to_string(root.join("Cargo.toml"))
            .map_err(|_| "provider-crate-layout-metadata-unavailable".to_owned())?;
        let mut members = Vec::new();
        let mut in_members = false;
        for line in workspace.lines() {
            let trimmed = line.trim();
            if trimmed == "members = [" {
                in_members = true;
                continue;
            }
            if !in_members {
                continue;
            }
            if trimmed == "]" {
                break;
            }
            let relative = trimmed.trim_end_matches(',').trim_matches('"');
            let manifest_path = root
                .join(relative)
                .join("Cargo.toml")
                .canonicalize()
                .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?;
            let manifest = fs::read_to_string(&manifest_path)
                .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?;
            let package_name = manifest
                .lines()
                .find_map(|line| line.trim().strip_prefix("name = \""))
                .and_then(|name| name.strip_suffix('"'))
                .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
                .to_owned();
            members.push(WorkspaceMember {
                package_name,
                crate_dir: manifest_path.parent().unwrap().to_owned(),
                manifest_path,
                declares_driver: manifest_declares_driver(&manifest),
            });
        }
        Ok(members)
    }

    fn check_fixture(root: &Path) -> Result<(), String> {
        let root = root.canonicalize().unwrap();
        check_members(&root, manifest_workspace_members(&root)?)
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_package(root: &Path, name: &str) {
        let package = root.join("packages").join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(
            package.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
    }

    /// Give one fixture package the declared dependency that makes it a
    /// per-type driver crate.
    fn declare_resource_types_dependency(crate_dir: &Path) {
        let manifest = crate_dir.join("Cargo.toml");
        let text = fs::read_to_string(&manifest).expect("read fixture manifest");
        fs::write(
            &manifest,
            format!(
                "{text}\n[dependencies]\nd2b-resource-types = {{ path = \"../d2b-resource-types\" }}\n"
            ),
        )
        .expect("write fixture manifest");
    }

    /// Classify one crate the way the check does: from its name and the
    /// dependency its manifest declares.
    fn classified_kind(root: &Path, name: &str) -> ProviderNameKind {
        let manifest = fs::read_to_string(root.join("packages").join(name).join("Cargo.toml"))
            .expect("read crate manifest");
        name_kind(name, manifest_declares_driver(&manifest))
    }

    fn required_readme(identity: &str) -> String {
        let mut readme = String::new();
        for section in REQUIRED_README_SECTIONS {
            readme.push_str(&format!("## {section}\n\n"));
            if *section == "Provider identity" {
                readme.push_str(&format!("| Provider name | `{identity}` |\n\n"));
            }
        }
        readme
    }

    #[test]
    fn conforming_tree_is_idempotent_and_non_provider_members_are_ignored() {
        let fixture = Fixture::new("clean");
        assert_eq!(check_fixture(&fixture.root), Ok(()));
        assert_eq!(check_fixture(&fixture.root), Ok(()));
    }

    #[test]
    fn the_provider_matrix_is_closed_and_has_two_bootstrap_rows() {
        assert_eq!(PROVIDER_MATRIX.len(), 27);

        let identities: BTreeSet<_> = PROVIDER_MATRIX
            .iter()
            .map(|row| row.identity)
            .collect();
        let crates: BTreeSet<_> = PROVIDER_MATRIX
            .iter()
            .map(|row| row.crate_name)
            .collect();
        assert_eq!(identities.len(), PROVIDER_MATRIX.len());
        assert_eq!(crates.len(), PROVIDER_MATRIX.len());
        assert_eq!(
            PROVIDER_MATRIX
                .iter()
                .filter(|row| row.bootstrap)
                .map(|row| row.identity)
                .collect::<Vec<_>>(),
            vec!["system-core", "system-minijail"]
        );
        for row in PROVIDER_MATRIX {
            // A crate is renamed with the family it realizes, so the identity
            // suffix is only required to be non-empty here; the provider
            // identity - and with it the dossier and the catalog id - is what
            // stays put across a rename.
            assert!(row.crate_name.strip_prefix(PROVIDER_PREFIX).is_some_and(|suffix| !suffix.is_empty()));
            assert!(row.bazel_target.ends_with(":all-tests"));
            assert!(row.dossier_path.ends_with(&format!(
                "ADR-046-provider-{}.md",
                row.identity
            )));
            assert!(row.source_path.starts_with("packages/"));
            assert!(row.test_path.starts_with("packages/"));
            assert!(matches!(row.unit, "U5" | "U6" | "U7" | "U8" | "U9" | "U10" | "U11" | "U12"));
        }
    }

    #[test]
    fn the_committed_tree_matches_every_provider_matrix_row() {
        let root = repo_root().expect("resolve repository root");
        let members = cargo_workspace_members(root).expect("read workspace metadata");
        assert_eq!(
            check_closed_matrix(root, &members),
            Ok(()),
            "the committed Provider matrix must have one live source, test, and dossier per row, and an aggregate target"
        );
    }

    #[test]
    fn every_provider_prefixed_name_has_one_explicit_classification() {
        let root = repo_root().expect("resolve repository root");
        let members = manifest_workspace_members(root).expect("read workspace manifest");
        let mut manifests: BTreeMap<String, PathBuf> = members
            .into_iter()
            .map(|member| (member.package_name, member.manifest_path))
            .filter(|(name, _)| name.starts_with(PROVIDER_PREFIX))
            .collect();
        for entry in fs::read_dir(root.join("packages")).expect("read packages directory") {
            let entry = entry.expect("read package entry");
            if entry.file_type().expect("read package entry type").is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(PROVIDER_PREFIX) {
                    manifests
                        .entry(name)
                        .or_insert_with(|| entry.path().join("Cargo.toml"));
                }
            }
        }

        assert!(
            !manifests.is_empty(),
            "Provider-name classification must inspect a non-empty scope"
        );
        for (name, manifest_path) in manifests {
            let manifest = fs::read_to_string(&manifest_path).expect("read crate manifest");
            let declares_driver = manifest_declares_driver(&manifest);
            match name_kind(&name, declares_driver) {
                ProviderNameKind::NonProvider => {
                    let single_segment = name
                        .strip_prefix(PROVIDER_PREFIX)
                        .is_some_and(|suffix| suffix.split('-').count() == 1);
                    assert!(
                        NON_PROVIDER_PREFIXED.contains(&name.as_str())
                            || single_segment
                            || declares_driver,
                        "{name} is neither an explicit non-Provider helper, a single-segment driver crate, nor a crate that declares a driver"
                    );
                }
                ProviderNameKind::Provider => {
                    assert!(
                        catalogued_provider(&name)
                            || (!declares_driver
                                && name
                                    .strip_prefix(PROVIDER_PREFIX)
                                    .is_some_and(|suffix| suffix.split('-').count() >= 2)),
                        "{name} is neither a catalogued Provider identity nor a two-segment identity that declares no driver"
                    );
                }
                ProviderNameKind::Malformed => {
                    assert!(
                        name.starts_with(PROVIDER_PREFIX),
                        "{name} is malformed but not Provider-prefixed"
                    );
                }
            }
        }
    }

    /// The driver signal is the dependency declaration, in the forms a Cargo
    /// manifest spells it, and nothing else in the manifest.
    #[test]
    fn the_driver_signal_reads_the_declared_resource_types_dependency() {
        assert!(manifest_declares_driver(
            "[dependencies]\nd2b-resource-types = { path = \"../d2b-resource-types\", version = \"0.0.0-bootstrap\" }\n"
        ));
        assert!(manifest_declares_driver(
            "[dependencies]\nresource-types = { package = \"d2b-resource-types\", path = \"../d2b-resource-types\" }\n"
        ));
        assert!(manifest_declares_driver(
            "[target.'cfg(unix)'.dependencies.d2b-resource-types]\npath = \"../d2b-resource-types\"\n"
        ));
        assert!(!manifest_declares_driver(
            "[dependencies]\nd2b-resource-types-extra = { path = \"../d2b-resource-types-extra\" }\n"
        ));
        assert!(!manifest_declares_driver(
            "# d2b-resource-types = { path = \"../d2b-resource-types\" }\n[dependencies]\n"
        ));
        assert!(!manifest_declares_driver(
            "[package]\nname = \"d2b-provider-example\"\ndescription = \"depends on d2b-resource-types\"\n"
        ));
    }

    /// A per-type driver crate is named after the resource type it serves, so
    /// its type name may contain a dash. The declared driver, not that shape,
    /// is what keeps it out of the packaging obligations and the catalog.
    #[test]
    fn a_per_type_driver_crate_with_a_dashed_type_name_needs_no_matrix_row() {
        let fixture = Fixture::new("dashed-driver");
        let driver = fixture.add_package("d2b-provider-wayland-policy");
        declare_resource_types_dependency(&driver);
        fixture.set_members(&[
            "d2b-core",
            "d2b-provider-fixture-example",
            "d2b-provider-wayland-policy",
        ]);

        assert_eq!(
            classified_kind(&fixture.root, "d2b-provider-wayland-policy"),
            ProviderNameKind::NonProvider
        );
        // A driver crate ships no packaging artifact, so it owes no Provider
        // layout.
        assert_eq!(check_fixture(&fixture.root), Ok(()));

        let members = manifest_workspace_members(&fixture.root).expect("read workspace manifest");
        let error = check_closed_matrix(&fixture.root, &members)
            .expect_err("the fixture holds no Provider matrix row");
        assert!(
            !error.contains("d2b-provider-wayland-policy"),
            "a declared driver needs no catalog row: {error}"
        );
    }

    /// The control for the dashed name above: the same name shape without the
    /// declared driver is a packaging Provider, so the check may not start
    /// passing a crate that genuinely forgot its matrix row.
    #[test]
    fn a_dashed_provider_name_without_a_driver_declaration_keeps_the_packaging_obligations() {
        let fixture = Fixture::new("dashed-provider");
        fixture.add_package("d2b-provider-wayland-policy");
        fixture.set_members(&[
            "d2b-core",
            "d2b-provider-fixture-example",
            "d2b-provider-wayland-policy",
        ]);

        assert_eq!(
            classified_kind(&fixture.root, "d2b-provider-wayland-policy"),
            ProviderNameKind::Provider
        );
        let error = check_fixture(&fixture.root).expect_err("a packaging Provider owes its layout");
        assert!(error.contains("missing-provider-crate-path"), "{error}");
        assert!(error.contains("d2b-provider-wayland-policy"), "{error}");

        let members = manifest_workspace_members(&fixture.root).expect("read workspace manifest");
        let error = check_closed_matrix(&fixture.root, &members)
            .expect_err("the fixture holds no Provider matrix row");
        assert!(
            error.contains(
                r#"{"error":"provider-matrix-row-unexpected","crate":"d2b-provider-wayland-policy"}"#
            ),
            "{error}"
        );
    }

    /// A provider-prefixed crate that declares no driver keeps the packaging
    /// identity however its name reads, so its catalog row is still demanded.
    #[test]
    fn a_packaging_provider_without_a_driver_declaration_keeps_its_matrix_row() {
        let fixture = Fixture::new("packaging-provider");
        fixture.add_package("d2b-provider-volume-local");

        assert_eq!(
            classified_kind(&fixture.root, "d2b-provider-volume-local"),
            ProviderNameKind::Provider
        );
        // Dropping the crate from the workspace must report its row missing
        // rather than silently pass.
        let members = manifest_workspace_members(&fixture.root).expect("read workspace manifest");
        let error = check_closed_matrix(&fixture.root, &members)
            .expect_err("the fixture holds no Provider matrix row");
        assert!(
            error.contains(
                r#"{"error":"provider-matrix-row-missing","crate":"d2b-provider-volume-local"}"#
            ),
            "{error}"
        );
    }

    /// The single-segment shape is unchanged: it names a per-type driver crate
    /// whether or not the manifest declares the resource types.
    #[test]
    fn a_single_segment_driver_crate_stays_a_non_provider() {
        let root = repo_root().expect("resolve repository root");
        assert_eq!(
            classified_kind(root, "d2b-provider-endpoint"),
            ProviderNameKind::NonProvider
        );
        assert_eq!(
            provider_name_kind("d2b-provider-endpoint", false),
            ProviderNameKind::NonProvider
        );
    }

    #[test]
    fn readme_only_integration_ratchet_is_exactly_the_scaffolded_set() {
        let expected = [
            "d2b-provider-activation-nixos",
            "d2b-provider-audio-pipewire",
            "d2b-provider-clipboard-wayland",
            "d2b-provider-credential-entra",
            "d2b-provider-credential-managed-identity",
            "d2b-provider-credential-secret-service",
            "d2b-provider-device-gpu",
            "d2b-provider-display-wayland",
            "d2b-provider-notification-desktop",
            "d2b-provider-process-minijail",
            "d2b-provider-process-systemd",
            "d2b-provider-guest-azure-container-apps",
            "d2b-provider-guest-azure-virtual-machine",
            "d2b-provider-guest-cloud-hypervisor",
            "d2b-provider-system-core",
            "d2b-provider-transport-azure-relay",
            "d2b-provider-transport-unix",
            "d2b-provider-volume-virtiofs",
        ];
        assert_eq!(
            README_ONLY_INTEGRATION_RATCHET, &expected,
            "README-only integration coverage must remain an explicit closed set"
        );
        let root = repo_root().expect("resolve repository root");
        for name in expected {
            let integration = root.join("packages").join(name).join("integration");
            assert!(
                integration.join("README.md").is_file(),
                "{name} must retain its integration scaffold README"
            );
            assert!(
                !integration_has_rust_scenario(&integration).expect("inspect integration scaffold"),
                "{name} must leave executable integration wiring to its owning implementation"
            );
        }
    }

    #[test]
    fn integration_readme_and_rust_scenario_are_both_required() {
        let fixture = Fixture::new("integration");
        fs::remove_file(fixture.provider_dir().join("integration/README.md")).unwrap();
        fs::remove_file(fixture.provider_dir().join("integration/scenario.rs")).unwrap();

        let error = check_fixture(&fixture.root).unwrap_err();
        eprintln!("synthetic perturbation rejected: {error}");
        assert_eq!(
            error,
            r#"{"error":"missing-provider-crate-path","crate":"d2b-provider-fixture-example","missing":["integration/*.rs","integration/README.md"]}"#
        );
    }

    #[test]
    fn an_on_disk_provider_omitted_from_workspace_is_rejected() {
        let fixture = Fixture::new("non-member");
        let omitted = fixture.add_package("d2b-provider-fixture-omitted");
        fs::create_dir_all(omitted.join("tests")).unwrap();
        fs::create_dir_all(omitted.join("integration")).unwrap();
        fs::write(
            omitted.join("README.md"),
            required_readme("fixture-omitted"),
        )
        .unwrap();

        let error = check_fixture(&fixture.root).unwrap_err();
        assert!(error.contains("provider-crate-not-workspace-member"));
        assert!(error.contains("d2b-provider-fixture-omitted"));
    }

    #[test]
    fn a_malformed_provider_name_is_rejected_instead_of_ignored() {
        let fixture = Fixture::new("malformed");
        fixture.add_package("d2b-provider-fixture-");
        fixture.set_members(&[
            "d2b-core",
            "d2b-provider-fixture-example",
            "d2b-provider-fixture-",
        ]);

        let error = check_fixture(&fixture.root).unwrap_err();
        assert!(error.contains("provider-crate-name-invalid"));
        assert!(error.contains("d2b-provider-fixture-"));
    }

    #[test]
    fn empty_provider_scope_fails_closed() {
        let fixture = Fixture::new("empty");
        fixture.set_members(&["d2b-core"]);
        assert_eq!(
            check_fixture(&fixture.root),
            Err(
                r#"{"error":"provider-crate-not-workspace-member","crate":"d2b-provider-fixture-example"}"#
                    .to_owned()
            )
        );
    }

    #[test]
    fn caller_supplied_workspace_paths_are_never_rendered() {
        let fixture = Fixture::new("redaction");
        let marker = format!("caller-secret-{}", std::process::id());
        fs::write(
            fixture.root.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\n    \"../{marker}\",\n]\n"),
        )
        .unwrap();
        let error = check_fixture(&fixture.root).unwrap_err();
        assert!(!error.contains(&marker));
    }

    /// A driver declared in a shared crate is refused, naming the module: the
    /// exemption list is what keeps the still-un-migrated families passing,
    /// and nothing else may declare a driver outside a provider crate.
    #[test]
    fn a_driver_declared_in_a_shared_crate_is_refused() {
        let fixture = Fixture::new("shared-driver");
        let broker = fixture.root.join("packages/d2b-broker/src");
        fs::create_dir_all(&broker).unwrap();
        fs::write(
            broker.join("hidden_driver.rs"),
            "impl ResourceDriver for HiddenDriver {}\n",
        )
        .unwrap();
        let error = check_shared_driver_placements(&fixture.root)
            .expect_err("a driver declared in a shared crate is refused");
        assert!(error.contains("shared-crate-driver-placement"), "{error}");
        assert!(
            error.contains("packages/d2b-broker/src/hidden_driver.rs"),
            "{error}"
        );

        fs::remove_file(broker.join("hidden_driver.rs")).unwrap();
        assert_eq!(check_shared_driver_placements(&fixture.root), Ok(()));
    }

    /// The exemption list matches the committed tree exactly: every entry
    /// names a module that still declares a driver, and no shared-crate module
    /// declares one without an entry. The ratchet holds no entries now that
    /// the Guest family has moved, so this fails the moment a driver is
    /// declared in a shared crate without an exemption beside it.
    #[test]
    fn the_shared_driver_exemptions_match_the_committed_tree() {
        let root = repo_root().expect("resolve repository root");
        assert_eq!(check_shared_driver_placements(root), Ok(()));
    }

    /// A matrix row whose module moved fails: the row's own citations are part
    /// of the identity, so a deleted module cannot stay in the table.
    #[test]
    fn a_matrix_row_that_cites_a_deleted_module_fails() {
        let fixture = Fixture::new("matrix-stale-source");
        fixture.add_package("d2b-provider-volume-local");
        fixture.set_members(&[
            "d2b-core",
            "d2b-provider-fixture-example",
            "d2b-provider-volume-local",
        ]);
        let members = manifest_workspace_members(&fixture.root).expect("read workspace manifest");
        let error = check_closed_matrix(&fixture.root, &members)
            .expect_err("a matrix row citing a missing module is refused");
        assert!(error.contains("provider-matrix-source-missing"), "{error}");
    }

    /// The declaration signal is the `ResourceDriver for` shape, not the line
    /// prefix: a visibility qualifier, an attribute, or another item on the
    /// line does not hide a driver, a commented-out example is not one, and
    /// the framework's blanket `DynResourceDriver` glue is not one either.
    #[test]
    fn the_driver_signal_reads_the_shape_not_the_line_prefix() {
        let text = "pub struct Wrapper; impl ResourceDriver for ProcessDriver {}\n\
                    /// impl ResourceDriver for DocsExample {}\n\
                    impl<D: ResourceDriver> DynResourceDriver for D {}\n";
        assert_eq!(
            driver_declarations(text),
            vec![(1, "ProcessDriver".to_owned())]
        );
    }

    /// Test code never ships, so a driver-shaped item behind `#[cfg(test)]`
    /// is not a placement decision while the production item after it is.
    #[test]
    fn test_module_fixtures_are_not_production_declarations() {
        let text = "#[cfg(test)]\nmod tests {\n    impl ResourceDriver for StubDriver {}\n}\n\n\
                    impl ResourceDriver for ProductionDriver {}\n";
        assert_eq!(
            driver_declarations(text),
            vec![(6, "ProductionDriver".to_owned())]
        );
    }

    /// A `#[cfg(test)] mod` nested in another module stops at its own closing
    /// brace, so the enclosing module's production items are still scanned.
    #[test]
    fn nested_test_modules_stop_at_their_own_close() {
        let text = "mod outer {\n    #[cfg(test)]\n    mod tests {\n        impl ResourceDriver for StubDriver {}\n    }\n\n    pub fn kept() {}\n}\n\nimpl ResourceDriver for ProductionDriver {}\n";
        assert_eq!(
            driver_declarations(text),
            vec![(10, "ProductionDriver".to_owned())]
        );
    }

    /// The framework's shared declaration-only metadata driver is the one
    /// allowed case under the framework roots; a per-resource driver parked
    /// beside it still fails.
    #[test]
    fn the_framework_metadata_driver_is_allowed_but_others_are_not() {
        let fixture = Fixture::new("framework-driver");
        let runtime = fixture.root.join("packages/d2b-resource-runtime/src");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(
            runtime.join("metadata.rs"),
            "#[async_trait::async_trait]\nimpl ResourceDriverFactory for MetadataDriverFactory {}\n\nimpl ResourceDriver for MetadataDriver {}\n",
        )
        .unwrap();
        assert_eq!(check_shared_driver_placements(&fixture.root), Ok(()));

        fs::write(
            runtime.join("parked.rs"),
            "impl ResourceDriver for ProcessDriver {}\n",
        )
        .unwrap();
        let error = check_shared_driver_placements(&fixture.root)
            .expect_err("a per-resource driver parked in the framework crate is refused");
        assert!(error.contains("shared-crate-driver-placement"), "{error}");
        assert!(
            error.contains("packages/d2b-resource-runtime/src/parked.rs"),
            "{error}"
        );
    }

    /// An allowance that names a declaration the tree no longer has fails, so
    /// the framework list cannot rot into an unmonitored hole.
    #[test]
    fn a_missing_framework_driver_declaration_is_a_stale_allowance() {
        let fixture = Fixture::new("framework-stale");
        fs::create_dir_all(fixture.root.join("packages/d2b-resource-runtime/src")).unwrap();
        let error = check_shared_driver_placements(&fixture.root)
            .expect_err("an allowance that names no declaration is stale");
        assert!(
            error.contains("stale-framework-driver-declaration"),
            "{error}"
        );
    }

    /// A comment that cites a module the tree no longer has fails with the
    /// citation and the sentence, inside a monitored root.
    #[test]
    fn a_comment_citing_a_deleted_module_is_refused_with_its_sentence() {
        let fixture = Fixture::new("dangling-citation");
        fixture.add_package("d2bd");
        let broker = fixture.root.join("packages/d2b-broker/src");
        fs::create_dir_all(&broker).unwrap();
        fs::write(
            broker.join("note.rs"),
            "//! CTAPHID session state lives in `d2bd::security_key`, not here.\n",
        )
        .unwrap();
        let error = check_dangling_citations(&fixture.root)
            .expect_err("a deleted module citation is refused");
        assert!(error.contains("dangling-comment-citation"), "{error}");
        assert!(error.contains("d2bd::security_key"), "{error}");
        assert!(error.contains("note.rs"), "{error}");
    }

    /// The mechanical shapes are removed, the rewrite rechecks clean, code is
    /// untouched, and a second fix run changes nothing.
    #[test]
    fn mechanical_citations_are_removed_and_recheck_clean() {
        let fixture = Fixture::new("mechanical-fix");
        fixture.add_package("d2bd");
        let broker = fixture.root.join("packages/d2b-broker/src");
        fs::create_dir_all(&broker).unwrap();
        let note = broker.join("note.rs");
        fs::write(
            &note,
            "//! The port refuses (see `d2bd::gone`) before dispatch.\n\
             //! Open it, see `packages/d2b-gone/src/port.rs`.\n\
             const NOTE: &str = \"packages/d2b-gone/src/port.rs\";\n",
        )
        .unwrap();
        let written = fix(&fixture.root).expect("mechanical citations are removed");
        assert_eq!(written, vec![fs::canonicalize(&note).unwrap()]);
        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            "//! The port refuses before dispatch.\n\
             //! Open it.\n\
             const NOTE: &str = \"packages/d2b-gone/src/port.rs\";\n"
        );
        assert_eq!(check_dangling_citations(&fixture.root), Ok(()));
        assert_eq!(fix(&fixture.root), Ok(Vec::new()));
    }

    /// A citation woven into a sentence, and a line whose predecessor
    /// continues into it, are reported and left for a human.
    #[test]
    fn embedded_and_wrapped_citations_are_reported_not_rewritten() {
        let fixture = Fixture::new("embedded-citation");
        fixture.add_package("d2bd");
        let broker = fixture.root.join("packages/d2b-broker/src");
        fs::create_dir_all(&broker).unwrap();
        let note = broker.join("note.rs");
        let original = "//! The port lives in `d2bd::gone`, not here.\n\
                        //! The envelope documents\n\
                        //! `packages/d2b-gone/src/port.rs`.\n";
        fs::write(&note, original).unwrap();
        let error = fix(&fixture.root).expect_err("embedded citations need a human");
        assert!(error.contains("dangling-comment-citation"), "{error}");
        assert!(error.contains("d2bd::gone"), "{error}");
        assert_eq!(fs::read_to_string(&note).unwrap(), original);
        assert!(check_dangling_citations(&fixture.root).is_err());
    }

    /// A wrapped bare-token line is not dropped, and a doc-link definition is
    /// repointed rather than removed: neither removal is mechanical.
    #[test]
    fn wrapped_and_definition_lines_are_not_mechanical() {
        let wrapped = "/// `packages/d2b-gone/src/port.rs`.";
        let start = wrapped.find("packages/").unwrap();
        let end = start + "packages/d2b-gone/src/port.rs".len();
        assert!(mechanical_rewrite(
            wrapped,
            0,
            Some("/// Open the port and hand off"),
            start,
            end
        )
        .is_none());

        let definition =
            "/// [`PrepareStateDir`]: crate::broker_wire::BrokerRequest::PrepareStateDir";
        let start = definition.find("crate::").unwrap();
        assert!(mechanical_rewrite(definition, 0, None, start, definition.len()).is_none());
    }
}
