use std::{env, process};

fn main() {
    match env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("nixling-exec-runner {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("nixling-exec-runner: service mode is not implemented in this build");
            process::exit(78);
        }
    }
}
