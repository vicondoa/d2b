//! `nixling-exec-runner` binary entry point.
//!
//! Two invocations are supported:
//! - `--version` prints the crate version and exits 0.
//! - `--serve-exec --slot NN` runs the per-slot detached supervisor (see
//!   [`service_mode`]).
//!
//! Any other invocation fails closed with exit 78 so a misconfigured unit can
//! never silently degrade into an unsupervised exec.

use std::{env, process};

mod service_mode;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--version") if args.len() == 1 => {
            println!("nixling-exec-runner {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--serve-exec") => match parse_slot(&args[1..]) {
            Some(slot) => service_mode::main_service(slot),
            None => {
                eprintln!("nixling-exec-runner: --serve-exec requires --slot <0..32>");
                64
            }
        },
        _ => {
            eprintln!("nixling-exec-runner: unsupported invocation");
            78
        }
    }
}

/// Parse exactly `--slot <NN>` from the remaining args.
fn parse_slot(rest: &[String]) -> Option<u32> {
    match rest {
        [flag, value] if flag == "--slot" => value.parse::<u32>().ok(),
        _ => None,
    }
}
