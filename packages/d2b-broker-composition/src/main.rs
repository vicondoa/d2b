use std::process::ExitCode;

use d2b_broker::runtime::{RunError, parse_command, run};
// The composition root is the broker's handler seam (KTD1): provider
// handler crates are linked here, and the mechanical routing rule is
// enforced here, never in the envelope. The committed catalog admits no
// operation to the in-broker leg this pass (fixture-exercised seam), so the
// startup verification below is the production face of that invariant. The
// first family whose caller audit records an in-broker leg assignment
// registers its handler at this point (through
// `d2b_broker_composition::seam::register_production_handlers`) and passes
// the table into the runtime's handler seam.
use d2b_broker_composition::seam;

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

    // Fail closed at startup: every committed operation must route to the
    // forward carrier this pass, and a future in-broker operation must
    // have a registered handler before the broker serves it.
    if let Err(violation) = seam::verify_startup_routing(&[]) {
        eprintln!("broker composition invariant violation: {violation}");
        return ExitCode::from(1);
    }

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