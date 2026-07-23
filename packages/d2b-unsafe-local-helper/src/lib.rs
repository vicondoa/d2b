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

pub fn run_tty_exec(args: &[String]) -> i32 {
    tty_exec::run(args)
}

#[derive(Debug)]
pub struct ShellSupervisorRunError;

pub fn run_shell_supervisor() -> Result<(), ShellSupervisorRunError> {
    shell_supervisor::run_shell_supervisor().map_err(|_| ShellSupervisorRunError)
}
