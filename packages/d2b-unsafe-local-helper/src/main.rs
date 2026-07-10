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
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(hide = true)]
    ScopeSupervisor,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::ScopeSupervisor) => {
            run_scope_supervisor().map_err(|_| "scope runtime failed")
        }
        None => ScopeRuntime::new(SystemdUserScopeManager)
            .map_err(|_| "helper runtime unavailable")
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
