use std::process::ExitCode;

use nixling_priv_broker::runtime::{RunError, parse_command, run};

fn main() -> ExitCode {
    // Enable RUST_LOG-driven env filter so the broker surfaces
    // detail-level spawn / live-handler failures in journalctl. Without
    // env_filter() the tracing subscriber only forwards INFO+ messages
    // with no context, and the daemon's "Broker.LiveHandlerFailed"
    // envelope is useless for live operator debugging.
    tracing_subscriber::fmt()
        .with_target(false)
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    match parse_command(std::env::args().skip(1)) {
        Ok(command) => match run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => report_error(err),
        },
        Err(err) => report_error(err),
    }
}

fn report_error(error: RunError) -> ExitCode {
    match error {
        RunError::Usage(message) => {
            eprintln!("usage error: {message}");
            ExitCode::from(2)
        }
        RunError::Io(error) => {
            eprintln!("broker io error: {error}");
            ExitCode::from(1)
        }
        RunError::Protocol(message) => {
            eprintln!("broker protocol error: {message}");
            ExitCode::from(3)
        }
    }
}
