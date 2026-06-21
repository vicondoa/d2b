use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "nixling-guest-shell-runner")]
#[command(about = "Internal nixling persistent-shell helper")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Daemon(DaemonArgs),
    Attach(AttachArgs),
    List(ManagementArgs),
    Detach(SessionManagementArgs),
    Kill(SessionManagementArgs),
}

#[derive(Debug, Parser)]
pub struct DaemonArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub home: PathBuf,
}

#[derive(Debug, Parser)]
pub struct AttachArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Parser)]
pub struct ManagementArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct SessionManagementArgs {
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub json: bool,
}
