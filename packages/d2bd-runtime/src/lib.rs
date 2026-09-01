//! Provider-independent daemon runtime services.
//!
//! This crate owns the daemon's transport, public wire, audit, metrics,
//! ComponentSession transport, lifecycle primitives, supervisor, and other
//! provider-neutral state. The `d2bd` crate remains the static composition
//! root for provider selection and effect adapters.

pub mod admission;
pub mod authority_persistence;
pub mod autostart;
pub mod broker_transport;
pub mod ch_api;
pub mod ch_stats;
pub mod component_session_vsock;
pub mod concurrency;
pub mod console_session;
pub mod daemon_audit;
pub mod daemon_client;
pub mod daemon_config;
pub mod daemon_version;
pub mod exec_detached;
pub mod exec_session;
pub mod exec_session_real;
pub mod exec_support;
pub mod guest_component_session;
pub mod guest_mode;
pub mod guest_resource_runtime;
pub mod host_mode;
pub mod json_io;
pub mod kernel_module_check;
pub mod known_hosts_refresh;
pub mod metrics;
pub mod otel_host_bridge_readiness;
pub mod ownership_preflight;
pub mod pidfs_probe;
pub mod public_projection;
pub mod public_read_model;
pub mod readiness;
pub mod resource_api;
pub mod resource_operator_activation;
pub mod resource_runtime_support;
pub mod resource_store_runtime;
pub mod runtime_capability;
pub mod runtime_process;
pub mod runtime_util;
pub mod shell_backend;
pub mod ssh_host_key_preflight;
pub mod supervisor;
pub mod target_runtime;
pub mod terminal_session;
pub mod typed_error;
pub mod typed_shell_targets;
pub mod unix_transport;
pub mod unsafe_local_helper;
pub mod unsafe_local_terminal;
pub mod usbipd_perenv_autostart;
pub mod vm_start_support;
pub mod wire;
pub mod wire_response_helpers;
pub mod workload_dispatch;
pub mod workload_target_index;
pub mod zone_authority;

#[cfg(test)]
pub(crate) fn test_scratch_root() -> std::path::PathBuf {
    std::env::var_os("TEST_TMPDIR")
        .or_else(|| std::env::var_os("CARGO_TARGET_TMPDIR"))
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .map(|path| path.join("target"))
        .expect("resolve test scratch root")
}
