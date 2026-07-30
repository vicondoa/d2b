//! Process entry point.
//!
//! Deliberately thin: it builds the async runtime, hands off to [`cli::main`],
//! and translates the result into a process exit status. All real work happens
//! in the library so that it can be unit tested without spawning a process.

use clap::Parser;

use d2b_agentterm::cli;

fn main() -> anyhow::Result<()> {
    // Parse before building the runtime, so `--help` and argument errors are
    // fast and do not spin up worker threads.
    let args = cli::Cli::parse();

    // A multi-thread runtime is used rather than the current-thread flavour
    // because the pump and the socket server run concurrently and a slow
    // client should never be able to delay terminal I/O.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let code = runtime.block_on(cli::main(args))?;

    // Drop the runtime before exiting. `std::process::exit` does not run
    // destructors, and the raw-mode guard in `tty` restores the terminal in
    // its `Drop` impl -- so exiting with the runtime still alive could leave
    // the user's terminal in raw mode. Dropping here forces that cleanup to
    // happen while the process is still running normally.
    drop(runtime);

    std::process::exit(code);
}
