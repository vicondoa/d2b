//! ComponentSession-owned unsafe-local user runtime.
//!
//! The parent control plane supplies the authenticated session and service
//! adapters. Direct execution must not recreate the removed helper protocol,
//! inherited TTY status channel, or supervisor bootstrap fallback.
fn main() {
    eprintln!("d2b-unsafe-local-helper: component-session-unavailable");
    std::process::exit(78);
}
