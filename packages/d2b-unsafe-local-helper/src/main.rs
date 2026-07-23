use clap::{Parser, Subcommand};
use d2b_unsafe_local_helper::protocol::{HelperClient, default_helper_socket_path};
use d2b_unsafe_local_helper::runtime::{ScopeRuntime, run_scope_supervisor};
use d2b_unsafe_local_helper::systemd::SystemdUserScopeManager;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "d2b-unsafe-local-helper")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, default_value_os_t = default_helper_socket_path().to_path_buf())]
    socket: PathBuf,
    #[arg(long, default_value = "d2bd")]
    daemon_user: String,
    #[arg(long, value_name = "NIX_STORE_BINARY")]
    wayland_proxy: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(hide = true)]
    ScopeSupervisor,
    #[command(hide = true)]
    ShellSupervisor,
}

fn main() {
    let raw = std::env::args().skip(1).collect::<Vec<_>>();
    if raw.first().map(String::as_str) == Some("--tty-exec") {
        std::process::exit(d2b_unsafe_local_helper::run_tty_exec(&raw[1..]));
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::ScopeSupervisor) => run_scope_supervisor().map_err(|error| {
            eprintln!("scope runtime failed: {error:?}");
            "scope runtime failed"
        }),
        Some(Command::ShellSupervisor) => {
            d2b_unsafe_local_helper::run_shell_supervisor().map_err(|_| "shell runtime failed")
        }
        None => cli
            .wayland_proxy
            .ok_or("immutable Wayland proxy path is required")
            .and_then(|proxy| {
                ScopeRuntime::new(SystemdUserScopeManager::new(), proxy)
                    .map_err(|_| "helper runtime unavailable")
            })
            .and_then(|runtime| {
                HelperClient::new(cli.socket, &cli.daemon_user, runtime)
                    .map_err(|_| "helper registration failed")
            })
            .and_then(|client| client.run().map_err(|_| "helper connection failed")),
    };
    if let Err(message) = result {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
