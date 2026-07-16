use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "d2b-guest-shell-runner")]
#[command(about = "Internal libshpool data-plane helper for d2b guest service")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Daemon(DaemonArgs),
    Attach(AttachArgs),
    List(ManagementArgs),
    Detach(SessionManagementArgs),
    Kill(SessionManagementArgs),
}

#[derive(Parser)]
pub struct DaemonArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub home: PathBuf,
}

#[derive(Parser)]
pub struct AttachArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser)]
pub struct ManagementArgs {
    #[arg(long)]
    pub socket: PathBuf,
}

#[derive(Parser)]
pub struct SessionManagementArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub name: String,
}
