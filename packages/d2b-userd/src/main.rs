use std::{env, process};

fn main() {
    match env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("d2b-userd {}", env!("CARGO_PKG_VERSION"));
        }
        None => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|_| process::exit(78));
            if let Err(error) = runtime.block_on(d2b_userd::runtime::run_production()) {
                process::exit(error.exit_code());
            }
        }
        Some(_) => process::exit(78),
    }
}
