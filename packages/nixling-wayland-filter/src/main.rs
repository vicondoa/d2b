//! Host-side Wayland filter proxy for nixling graphics VMs.
//!
//! Startup sequence (fail-closed):
//!   1. Parse CLI arguments.
//!   2. Resolve filter policy from defaults + operator overrides.
//!      Exit non-zero on policy parse errors.
//!   3. Connect to upstream compositor (--connect).
//!      Exit non-zero if the connection fails.
//!   4. Create the listen socket at --listen.
//!      Exit non-zero if binding fails.
//!      The listen socket is NEVER created before upstream connects.
//!   5. Enter the dispatch loop.

use std::{
    cell::RefCell,
    io,
    os::{
        fd::OwnedFd,
        unix::net::UnixListener,
    },
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use clap::Parser;
use env_logger::Env;
use nixling_wayland_filter::filter::{FilterClientHandler, FilterStateHandler, build_state};
use nixling_wayland_filter::{
    diag::DiagRateLimiter,
    policy::{FilterPolicy, PolicyInput},
};

#[derive(Parser, Debug)]
#[command(name = "nixling-wayland-filter")]
#[command(about = "Host-side Wayland filter proxy for nixling graphics VMs")]
struct Args {
    /// Path of the Unix socket to create and accept client connections on.
    #[arg(long)]
    listen: PathBuf,

    /// Path of the upstream host compositor socket to connect to.
    #[arg(long)]
    connect: String,

    /// VM name, e.g. `work`. Used in app-id prefix, title prefix, and logs.
    #[arg(long, value_name = "VM")]
    vm_name: String,

    /// Override the xdg_toplevel app-id prefix (default: `nixling.<vm>.`).
    #[arg(long)]
    app_id_prefix: Option<String>,

    /// Override the xdg_toplevel title prefix (default: `[<vm>] `).
    #[arg(long)]
    title_prefix: Option<String>,

    /// Explicitly deny a global not denied by default (repeatable).
    #[arg(long = "deny-global", value_name = "INTERFACE")]
    deny_globals: Vec<String>,

    /// Explicitly allow a global not allowed by default (repeatable).
    #[arg(long = "allow-global", value_name = "INTERFACE")]
    allow_globals: Vec<String>,

    /// Set a per-global version cap in `INTERFACE=VERSION` form (repeatable).
    #[arg(long = "max-version", value_name = "INTERFACE=VERSION")]
    max_versions: Vec<String>,

    /// Emit a log line for every global filtered from advertisement.
    #[arg(long)]
    log_filtered_globals: bool,
}

fn parse_max_version(s: &str) -> Result<(String, u32), String> {
    let (iface, ver_str) = s
        .split_once('=')
        .ok_or_else(|| format!("expected INTERFACE=VERSION, got `{s}`"))?;
    let ver: u32 = ver_str
        .parse()
        .map_err(|_| format!("invalid version `{ver_str}` in `{s}`"))?;
    Ok((iface.to_owned(), ver))
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    // Parse max-version overrides early so we can fail fast.
    let max_versions: Vec<(String, u32)> = args
        .max_versions
        .iter()
        .map(|s| parse_max_version(s))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| {
            eprintln!("nixling-wayland-filter: error in --max-version: {e}");
            std::process::exit(1);
        });

    let input = PolicyInput {
        vm_name: args.vm_name.clone(),
        app_id_prefix: args.app_id_prefix.clone(),
        title_prefix: args.title_prefix.clone(),
        deny_globals: args.deny_globals.clone(),
        allow_globals: args.allow_globals.clone(),
        max_versions,
        log_filtered_globals: args.log_filtered_globals,
    };

    // Step 2: resolve policy.
    let policy = FilterPolicy::build(input);

    // Emit advisory warnings to stderr so they appear in the journal.
    for w in &policy.warnings {
        eprintln!("nixling-wayland-filter: warning: {}", w.message());
    }

    let policy = Rc::new(policy);

    // Step 3: connect to upstream compositor.
    let state = match build_state(&args.connect) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "nixling-wayland-filter: failed to connect to upstream compositor `{}`: {e}",
                args.connect
            );
            std::process::exit(1);
        }
    };

    let diag = Rc::new(RefCell::new(DiagRateLimiter::new(policy.vm_name.clone())));

    // Install state handler to set up per-client handlers on accept.
    state.set_handler(FilterStateHandler::new(policy.clone(), diag.clone()));

    // Step 4: create the listen socket AFTER successful upstream connect.
    let listen_path = &args.listen;

    // Remove a stale socket if present so restart cycles are idempotent.
    if listen_path.exists() {
        if let Err(e) = std::fs::remove_file(listen_path) {
            eprintln!(
                "nixling-wayland-filter: failed to remove stale socket `{}`: {e}",
                listen_path.display()
            );
            std::process::exit(1);
        }
    }

    let listener = match UnixListener::bind(listen_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "nixling-wayland-filter: failed to bind listen socket `{}`: {e}",
                listen_path.display()
            );
            std::process::exit(1);
        }
    };

    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!(
            "nixling-wayland-filter: failed to set listen socket non-blocking: {e}"
        );
        std::process::exit(1);
    }

    log::info!(
        "[nixling-wlproxy] vm={} listening on {} upstream={}",
        args.vm_name,
        listen_path.display(),
        args.connect
    );

    // Step 5: dispatch loop.
    run_loop(&state, &listener, &policy.vm_name, &diag);
}

fn run_loop(
    state: &Rc<wl_proxy::state::State>,
    listener: &UnixListener,
    vm: &str,
    diag: &Rc<RefCell<DiagRateLimiter>>,
) {
    let mut last_diag_flush = Instant::now();
    loop {
        // Accept all pending new client connections (non-blocking).
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let owned_fd: OwnedFd = stream.into();
                    let owned_fd = Rc::new(owned_fd);
                    match state.add_client(&owned_fd) {
                        Ok(client) => {
                            client.set_handler(FilterClientHandler::new(vm.to_owned()));
                        }
                        Err(e) => {
                            log::warn!(
                                "[nixling-wlproxy] vm={vm} failed to add client: {e}"
                            );
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if is_recoverable_accept_error(&e) => {
                    log::warn!("[nixling-wlproxy] vm={vm} recoverable accept error: {e}");
                    continue;
                }
                Err(e) => {
                    eprintln!("nixling-wayland-filter: accept error: {e}");
                    std::process::exit(1);
                }
            }
        }

        // Dispatch all pending server and client messages, waiting up to 10 ms.
        match state.dispatch(Some(Duration::from_millis(10))) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("nixling-wayland-filter: dispatch error: {e}");
                std::process::exit(1);
            }
        }

        if last_diag_flush.elapsed() >= Duration::from_secs(60) {
            diag.borrow_mut().flush_suppressed();
            last_diag_flush = Instant::now();
        }
    }
}

fn is_recoverable_accept_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Interrupted {
        return true;
    }

    matches!(
        error.raw_os_error(),
        Some(libc::ECONNABORTED | libc::EINTR)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_accept_is_recoverable() {
        let err = io::Error::from_raw_os_error(libc::EINTR);
        assert!(is_recoverable_accept_error(&err));
    }

    #[test]
    fn aborted_accept_is_recoverable() {
        let err = io::Error::from_raw_os_error(libc::ECONNABORTED);
        assert!(is_recoverable_accept_error(&err));
    }

    #[test]
    fn permission_denied_accept_is_fatal() {
        let err = io::Error::from_raw_os_error(libc::EACCES);
        assert!(!is_recoverable_accept_error(&err));
    }
}
