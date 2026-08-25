const RUNTIME_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../Cargo.toml")),
    ("BUILD.bazel", include_str!("../BUILD.bazel")),
    ("src/admission.rs", include_str!("../src/admission.rs")),
    (
        "src/authority_persistence.rs",
        include_str!("../src/authority_persistence.rs"),
    ),
    ("src/autostart.rs", include_str!("../src/autostart.rs")),
    (
        "src/broker_transport.rs",
        include_str!("../src/broker_transport.rs"),
    ),
    ("src/ch_api.rs", include_str!("../src/ch_api.rs")),
    ("src/ch_stats.rs", include_str!("../src/ch_stats.rs")),
    ("src/concurrency.rs", include_str!("../src/concurrency.rs")),
    (
        "src/console_session.rs",
        include_str!("../src/console_session.rs"),
    ),
    (
        "src/daemon_audit.rs",
        include_str!("../src/daemon_audit.rs"),
    ),
    (
        "src/daemon_client.rs",
        include_str!("../src/daemon_client.rs"),
    ),
    (
        "src/daemon_config.rs",
        include_str!("../src/daemon_config.rs"),
    ),
    (
        "src/daemon_version.rs",
        include_str!("../src/daemon_version.rs"),
    ),
    (
        "src/exec_detached.rs",
        include_str!("../src/exec_detached.rs"),
    ),
    (
        "src/exec_session.rs",
        include_str!("../src/exec_session.rs"),
    ),
    (
        "src/exec_session_real.rs",
        include_str!("../src/exec_session_real.rs"),
    ),
    (
        "src/exec_support.rs",
        include_str!("../src/exec_support.rs"),
    ),
    (
        "src/component_session_vsock.rs",
        include_str!("../src/component_session_vsock.rs"),
    ),
    ("src/guest_mode.rs", include_str!("../src/guest_mode.rs")),
    ("src/host_mode.rs", include_str!("../src/host_mode.rs")),
    ("src/json_io.rs", include_str!("../src/json_io.rs")),
    (
        "src/kernel_module_check.rs",
        include_str!("../src/kernel_module_check.rs"),
    ),
    (
        "src/known_hosts_refresh.rs",
        include_str!("../src/known_hosts_refresh.rs"),
    ),
    ("src/lib.rs", include_str!("../src/lib.rs")),
    ("src/metrics.rs", include_str!("../src/metrics.rs")),
    (
        "src/otel_host_bridge_readiness.rs",
        include_str!("../src/otel_host_bridge_readiness.rs"),
    ),
    (
        "src/ownership_preflight.rs",
        include_str!("../src/ownership_preflight.rs"),
    ),
    ("src/pidfs_probe.rs", include_str!("../src/pidfs_probe.rs")),
    (
        "src/public_projection.rs",
        include_str!("../src/public_projection.rs"),
    ),
    (
        "src/public_read_model.rs",
        include_str!("../src/public_read_model.rs"),
    ),
    ("src/readiness.rs", include_str!("../src/readiness.rs")),
    (
        "src/resource_api.rs",
        include_str!("../src/resource_api.rs"),
    ),
    (
        "src/resource_operator_activation.rs",
        include_str!("../src/resource_operator_activation.rs"),
    ),
    (
        "src/resource_runtime_support.rs",
        include_str!("../src/resource_runtime_support.rs"),
    ),
    (
        "src/resource_store_runtime.rs",
        include_str!("../src/resource_store_runtime.rs"),
    ),
    (
        "src/runtime_capability.rs",
        include_str!("../src/runtime_capability.rs"),
    ),
    (
        "src/runtime_process.rs",
        include_str!("../src/runtime_process.rs"),
    ),
    (
        "src/runtime_util.rs",
        include_str!("../src/runtime_util.rs"),
    ),
    (
        "src/shell_backend.rs",
        include_str!("../src/shell_backend.rs"),
    ),
    (
        "src/ssh_host_key_preflight.rs",
        include_str!("../src/ssh_host_key_preflight.rs"),
    ),
    (
        "src/supervisor/dag.rs",
        include_str!("../src/supervisor/dag.rs"),
    ),
    (
        "src/supervisor/mod.rs",
        include_str!("../src/supervisor/mod.rs"),
    ),
    (
        "src/supervisor/pidfd.rs",
        include_str!("../src/supervisor/pidfd.rs"),
    ),
    (
        "src/supervisor/pidfd_table.rs",
        include_str!("../src/supervisor/pidfd_table.rs"),
    ),
    (
        "src/supervisor/readiness_liveness.rs",
        include_str!("../src/supervisor/readiness_liveness.rs"),
    ),
    (
        "src/supervisor/state.rs",
        include_str!("../src/supervisor/state.rs"),
    ),
    (
        "src/terminal_session.rs",
        include_str!("../src/terminal_session.rs"),
    ),
    (
        "src/target_runtime.rs",
        include_str!("../src/target_runtime.rs"),
    ),
    ("src/typed_error.rs", include_str!("../src/typed_error.rs")),
    (
        "src/typed_shell_targets.rs",
        include_str!("../src/typed_shell_targets.rs"),
    ),
    (
        "src/unix_transport.rs",
        include_str!("../src/unix_transport.rs"),
    ),
    (
        "src/unsafe_local_helper.rs",
        include_str!("../src/unsafe_local_helper.rs"),
    ),
    (
        "src/unsafe_local_terminal.rs",
        include_str!("../src/unsafe_local_terminal.rs"),
    ),
    (
        "src/usbipd_perenv_autostart.rs",
        include_str!("../src/usbipd_perenv_autostart.rs"),
    ),
    (
        "src/vm_start_support.rs",
        include_str!("../src/vm_start_support.rs"),
    ),
    ("src/wire.rs", include_str!("../src/wire.rs")),
    (
        "src/wire_response_helpers.rs",
        include_str!("../src/wire_response_helpers.rs"),
    ),
    (
        "src/workload_dispatch.rs",
        include_str!("../src/workload_dispatch.rs"),
    ),
    (
        "src/workload_target_index.rs",
        include_str!("../src/workload_target_index.rs"),
    ),
    (
        "src/zone_authority.rs",
        include_str!("../src/zone_authority.rs"),
    ),
];

#[test]
fn runtime_has_no_provider_implementation_dependency() {
    for (path, text) in RUNTIME_FILES {
        assert!(
            !text.contains("d2b-provider-") && !text.contains("d2b_provider_"),
            "provider implementation dependency in {path}"
        );
    }
}

#[test]
fn runtime_uses_narrow_direct_contracts() {
    let manifest = include_str!("../Cargo.toml");
    for dependency in [
        "d2b-contracts",
        "d2b-contracts-broker",
        "d2b-contracts-control",
        "d2b-contracts-provider",
        "d2b-contracts-resource",
        "d2b-contracts-zone-session",
    ] {
        assert!(
            manifest.contains(dependency),
            "missing direct dependency {dependency}"
        );
    }
}
