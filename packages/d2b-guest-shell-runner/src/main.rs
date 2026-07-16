use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use d2b_guest_shell_runner::{
    cli::{Cli, Command},
    name::validate_shell_name,
    socket::ExternalDataPlaneSocket,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Daemon(args) => {
            let socket = ExternalDataPlaneSocket::new(args.socket)?;
            if !args.home.is_absolute() {
                anyhow::bail!("libshpool home path must be absolute");
            }
            run_libshpool_daemon(&socket, &args.home)?;
        }
        Command::Attach(args) => {
            let socket = ExternalDataPlaneSocket::new(args.socket)?;
            validate_shell_name(&args.name)?;
            run_libshpool_attach(&socket, &args.name, args.force)?;
        }
        Command::List(args) => {
            let socket = ExternalDataPlaneSocket::new(args.socket)?;
            run_libshpool_list(&socket)?;
        }
        Command::Detach(args) => {
            let socket = ExternalDataPlaneSocket::new(args.socket)?;
            validate_shell_name(&args.name)?;
            run_libshpool_detach(&socket, &args.name)?;
        }
        Command::Kill(args) => {
            let socket = ExternalDataPlaneSocket::new(args.socket)?;
            validate_shell_name(&args.name)?;
            run_libshpool_kill(&socket, &args.name)?;
        }
    }
    Ok(())
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_daemon(socket: &ExternalDataPlaneSocket, home: &std::path::Path) -> Result<()> {
    d2b_guest_shell_runner::libshpool_bridge::daemon(socket, home)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_daemon(_socket: &ExternalDataPlaneSocket, _home: &std::path::Path) -> Result<()> {
    backend_unavailable()
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_attach(socket: &ExternalDataPlaneSocket, name: &str, force: bool) -> Result<()> {
    d2b_guest_shell_runner::libshpool_bridge::attach(socket, name, force)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_attach(
    _socket: &ExternalDataPlaneSocket,
    _name: &str,
    _force: bool,
) -> Result<()> {
    backend_unavailable()
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_list(socket: &ExternalDataPlaneSocket) -> Result<()> {
    d2b_guest_shell_runner::libshpool_bridge::list(socket)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_list(_socket: &ExternalDataPlaneSocket) -> Result<()> {
    backend_unavailable()
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_detach(socket: &ExternalDataPlaneSocket, name: &str) -> Result<()> {
    d2b_guest_shell_runner::libshpool_bridge::detach(socket, name)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_detach(_socket: &ExternalDataPlaneSocket, _name: &str) -> Result<()> {
    backend_unavailable()
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_kill(socket: &ExternalDataPlaneSocket, name: &str) -> Result<()> {
    d2b_guest_shell_runner::libshpool_bridge::kill(socket, name)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_kill(_socket: &ExternalDataPlaneSocket, _name: &str) -> Result<()> {
    backend_unavailable()
}

#[cfg(not(feature = "real-libshpool"))]
fn backend_unavailable<T>() -> Result<T> {
    anyhow::bail!("retained shell libshpool backend is unavailable")
}
