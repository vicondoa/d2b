//! Runner process lifecycle metadata for host-side argv generation.
//!
//! The table in this module is metadata only: it does not carry argv,
//! environment, QMP, fd, or path data. Per-role generators remain the
//! source of truth for command construction.

use nixling_core::processes::ProcessRole;

/// High-level lifecycle bucket for a process role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLifecycleClass {
    /// Long-lived runner/sidecar spawned through broker SpawnRunner.
    Spawnable,
    /// Readiness probe that does not own a long-lived runner argv.
    ReadinessOnly,
    /// Pre-start hook executed before its long-lived sidecar.
    PreStart,
    /// Host reconciliation step rather than a spawnable runner.
    HostReconcile,
}

impl RunnerLifecycleClass {
    pub const fn is_spawnable(self) -> bool {
        matches!(self, Self::Spawnable)
    }
}

/// Current regenerator wiring state for a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegeneratorWiring {
    /// `runner_argv_regenerator` invokes this role's typed generator.
    Wired,
    /// The role is intentionally classified but not dispatched by the
    /// regenerator yet.
    NotYetWired,
}

/// Static, non-sensitive metadata for one [`ProcessRole`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerProcessMetadata {
    pub role: ProcessRole,
    pub lifecycle: RunnerLifecycleClass,
    pub argv_generator_module: Option<&'static str>,
    pub test_coverage_label: &'static str,
    pub regenerator_wiring: RegeneratorWiring,
}

impl RunnerProcessMetadata {
    pub const fn spawnable(&self) -> bool {
        self.lifecycle.is_spawnable()
    }

    pub const fn regenerator_wired(&self) -> bool {
        matches!(self.regenerator_wiring, RegeneratorWiring::Wired)
    }
}

use ProcessRole::*;

/// One metadata row for every current process role.
pub const RUNNER_PROCESS_MATRIX: &[RunnerProcessMetadata] = &[
    row(
        HostReconcile,
        RunnerLifecycleClass::HostReconcile,
        None,
        "runner_process::host_reconcile",
        RegeneratorWiring::NotYetWired,
    ),
    row(
        StoreVirtiofsPreflight,
        RunnerLifecycleClass::ReadinessOnly,
        None,
        "runner_process::store_virtiofs_preflight",
        RegeneratorWiring::NotYetWired,
    ),
    row(
        SwtpmPreStartFlush,
        RunnerLifecycleClass::PreStart,
        None,
        "runner_process::swtpm_pre_start_flush",
        RegeneratorWiring::NotYetWired,
    ),
    row(
        Swtpm,
        RunnerLifecycleClass::Spawnable,
        Some("swtpm_argv"),
        "swtpm_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        Virtiofsd,
        RunnerLifecycleClass::Spawnable,
        Some("virtiofsd_argv"),
        "virtiofsd_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        Video,
        RunnerLifecycleClass::Spawnable,
        Some("video_argv"),
        "video_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        Gpu,
        RunnerLifecycleClass::Spawnable,
        Some("gpu_argv"),
        "gpu_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        GpuRenderNode,
        RunnerLifecycleClass::Spawnable,
        Some("gpu_argv"),
        "gpu_argv::render_node",
        RegeneratorWiring::Wired,
    ),
    row(
        Audio,
        RunnerLifecycleClass::Spawnable,
        Some("audio_argv"),
        "audio_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        CloudHypervisorRunner,
        RunnerLifecycleClass::Spawnable,
        Some("ch_argv"),
        "ch_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        QemuMediaRunner,
        RunnerLifecycleClass::Spawnable,
        Some("qemu_media_argv"),
        "qemu_media_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        VsockRelay,
        RunnerLifecycleClass::Spawnable,
        Some("vsock_relay_argv"),
        "vsock_relay_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        OtelHostBridge,
        RunnerLifecycleClass::Spawnable,
        Some("otel_host_bridge_argv"),
        "otel_host_bridge_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        GuestSshReadiness,
        RunnerLifecycleClass::ReadinessOnly,
        None,
        "runner_process::guest_ssh_readiness",
        RegeneratorWiring::NotYetWired,
    ),
    row(
        GuestControlHealth,
        RunnerLifecycleClass::ReadinessOnly,
        None,
        "runner_process::guest_control_health",
        RegeneratorWiring::NotYetWired,
    ),
    row(
        Usbip,
        RunnerLifecycleClass::Spawnable,
        Some("usbip_argv"),
        "usbip_argv",
        RegeneratorWiring::Wired,
    ),
    row(
        WaylandProxy,
        RunnerLifecycleClass::Spawnable,
        Some("wayland_proxy_argv"),
        "wayland_proxy_argv",
        RegeneratorWiring::NotYetWired,
    ),
];

const fn row(
    role: ProcessRole,
    lifecycle: RunnerLifecycleClass,
    argv_generator_module: Option<&'static str>,
    test_coverage_label: &'static str,
    regenerator_wiring: RegeneratorWiring,
) -> RunnerProcessMetadata {
    RunnerProcessMetadata {
        role,
        lifecycle,
        argv_generator_module,
        test_coverage_label,
        regenerator_wiring,
    }
}

/// Exhaustive role-to-metadata lookup. Adding a new [`ProcessRole`] must update
/// this match and [`RUNNER_PROCESS_MATRIX`].
pub const fn runner_process_metadata(role: &ProcessRole) -> &'static RunnerProcessMetadata {
    match role {
        HostReconcile => &RUNNER_PROCESS_MATRIX[0],
        StoreVirtiofsPreflight => &RUNNER_PROCESS_MATRIX[1],
        SwtpmPreStartFlush => &RUNNER_PROCESS_MATRIX[2],
        Swtpm => &RUNNER_PROCESS_MATRIX[3],
        Virtiofsd => &RUNNER_PROCESS_MATRIX[4],
        Video => &RUNNER_PROCESS_MATRIX[5],
        Gpu => &RUNNER_PROCESS_MATRIX[6],
        GpuRenderNode => &RUNNER_PROCESS_MATRIX[7],
        Audio => &RUNNER_PROCESS_MATRIX[8],
        CloudHypervisorRunner => &RUNNER_PROCESS_MATRIX[9],
        QemuMediaRunner => &RUNNER_PROCESS_MATRIX[10],
        VsockRelay => &RUNNER_PROCESS_MATRIX[11],
        OtelHostBridge => &RUNNER_PROCESS_MATRIX[12],
        GuestSshReadiness => &RUNNER_PROCESS_MATRIX[13],
        GuestControlHealth => &RUNNER_PROCESS_MATRIX[14],
        Usbip => &RUNNER_PROCESS_MATRIX[15],
        WaylandProxy => &RUNNER_PROCESS_MATRIX[16],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ROLES: &[ProcessRole] = &[
        HostReconcile,
        StoreVirtiofsPreflight,
        SwtpmPreStartFlush,
        Swtpm,
        Virtiofsd,
        Video,
        Gpu,
        GpuRenderNode,
        Audio,
        CloudHypervisorRunner,
        QemuMediaRunner,
        VsockRelay,
        OtelHostBridge,
        GuestSshReadiness,
        GuestControlHealth,
        Usbip,
        WaylandProxy,
    ];

    #[test]
    fn runner_process_matrix_has_one_row_for_every_process_role() {
        assert_eq!(RUNNER_PROCESS_MATRIX.len(), ALL_ROLES.len());
        for role in ALL_ROLES {
            let rows = RUNNER_PROCESS_MATRIX
                .iter()
                .filter(|row| &row.role == role)
                .count();
            assert_eq!(rows, 1, "expected exactly one matrix row for {role:?}");
            assert_eq!(runner_process_metadata(role).role, *role);
        }
    }

    #[test]
    fn runner_process_matrix_names_generator_and_test_labels() {
        for row in RUNNER_PROCESS_MATRIX {
            assert!(
                !row.test_coverage_label.is_empty(),
                "missing test label for {:?}",
                row.role
            );
            if row.spawnable() {
                assert!(
                    row.argv_generator_module.is_some(),
                    "spawnable role {:?} needs generator ownership",
                    row.role
                );
            } else {
                assert!(
                    row.argv_generator_module.is_none(),
                    "non-spawnable role {:?} should not name argv owner",
                    row.role
                );
            }
        }

        let qemu = runner_process_metadata(&QemuMediaRunner);
        assert_eq!(qemu.argv_generator_module, Some("qemu_media_argv"));
        assert_eq!(qemu.test_coverage_label, "qemu_media_argv");
    }
}
