use std::{env, process};

fn main() {
    match env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("d2b-userd {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("d2b-userd: service mode is not implemented in this build");
            process::exit(78);
        }
    }
}
