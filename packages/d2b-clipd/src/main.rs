// SPDX-License-Identifier: Apache-2.0
//! The d2b clipboard service process.
//!
//! The process accepts only the three inherited systemd SEQPACKET listeners.
//! It never discovers ambient paths or recreates the retired newline protocol.

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(error) = d2b_clipd::daemon::run() {
        eprintln!("d2b-clipd: {error}");
        std::process::exit(error.exit_code());
    }
}
