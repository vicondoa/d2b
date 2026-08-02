use super::NativeCommand;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version)]
pub(super) struct NativeCli {
    #[command(subcommand)]
    pub(super) command: NativeCommand,
}
