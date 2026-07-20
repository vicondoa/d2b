pub mod environment;
mod output_ring;
// The runtime component still type-checks its frozen migration shim, but the
// composed crate no longer exposes that helper/supervisor bootstrap surface.
#[allow(dead_code)]
mod runtime;
pub mod server;
pub mod services;
pub mod shell_runtime;
mod shell_socket;
mod shell_supervisor;
mod supervisor_protocol;
pub mod systemd;
mod tty_exec;

#[cfg(test)]
pub(crate) fn test_scratch_root() -> std::path::PathBuf {
    std::env::var_os("D2B_VALIDATION_SOCKET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(std::path::Path::parent)
                .expect("repository root")
                .to_path_buf()
        })
}

#[cfg(test)]
pub(crate) fn test_socket_root() -> std::path::PathBuf {
    std::env::var_os("D2B_VALIDATION_SOCKET_DIR")
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub use services::{
    AuthenticatedRuntimeSession, CompositionError, RecoveredShell, RuntimeComposition,
    RuntimeLifecycleState,
};

// Keep the frozen fail-closed component shims type-checked without exposing
// either legacy entrypoint from the composed library or binary.
const _: fn() -> Result<(), shell_supervisor::ShellSupervisorError> =
    shell_supervisor::run_shell_supervisor;
const _: fn(&[String]) -> i32 = tty_exec::run;
