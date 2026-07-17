// SPDX-License-Identifier: Apache-2.0
//! The d2b clipboard service process.
//!
//! Endpoint discovery and ComponentSession establishment are supplied by the
//! parent control plane. Starting this binary without that inherited adapter
//! must not recreate the removed pathname sockets or picker child protocol.

fn main() {
    eprintln!("d2b-clipd: clipboard-session-unavailable");
    std::process::exit(78);
}
