use std::{env, process};

fn main() {
    match env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("nixling-userd {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("nixling-userd: service mode is not implemented in this build");
            process::exit(78);
        }
    }
}
