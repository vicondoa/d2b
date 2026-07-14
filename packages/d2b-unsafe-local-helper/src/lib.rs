pub mod environment;
mod output_ring;
pub mod protocol;
pub mod runtime;
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

pub fn run_tty_exec(args: &[String]) -> i32 {
    tty_exec::run(args)
}

#[derive(Debug)]
pub struct ShellSupervisorRunError;

pub fn run_shell_supervisor() -> Result<(), ShellSupervisorRunError> {
    shell_supervisor::run_shell_supervisor().map_err(|_| ShellSupervisorRunError)
}
