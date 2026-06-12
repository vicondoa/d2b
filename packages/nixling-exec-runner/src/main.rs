//! `nixling-exec-runner` binary entry point.
//!
//! Three invocations are supported:
//! - `--version` prints the crate version and exits 0.
//! - `--serve-exec --slot NN` runs the per-slot detached supervisor (see
//!   [`service_mode`]).
//! - `--tty-exec --rows R --cols C -- <argv...>` is the interactive TTY exec
//!   helper: it makes itself the session leader with the inherited PTY slave as
//!   its controlling terminal, then `exec`s the target (see [`tty_helper`]).
//!
//! Any other invocation fails closed with exit 78 so a misconfigured unit can
//! never silently degrade into an unsupervised exec.

use std::{env, process};

mod service_mode;
mod tty_helper;

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
        // Interactive TTY exec helper (W14). guestd spawns this with the PTY
        // slave on stdin and a CLOEXEC status pipe on stdout; the helper makes
        // itself the session leader with the slave as its controlling terminal,
        // then exec's the target. See `tty_helper`.
        Some("--tty-exec") => tty_helper::run(&args[1..]),
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
