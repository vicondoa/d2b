use clap::Parser;

use d2b_agentterm::cli;

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let code = runtime.block_on(cli::main(args))?;

    // Drop the runtime before exiting so the raw-mode guard in `tty` runs its
    // destructor while the process is still alive.
    drop(runtime);

    std::process::exit(code);
}
