use std::process::ExitCode;

use nixling_priv_broker::runtime::{parse_command, run, RunError};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_target(false)
        .without_time()
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
