//! Host-side Wayland proxy for d2b graphics VMs.
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
    os::unix::net::UnixListener,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use clap::Parser;
use d2b_wayland_proxy::filter::{
    FilterClientHandler, FilterStateHandler, VirtualClipboardState, build_state,
    install_client_handlers,
};
use d2b_wayland_proxy::{
    bridge::{BridgeConfig, BridgeReconnectPolicy},
    diag::{DiagRateLimiter, bounded_error_detail},
    dmabuf::{DmabufFilter, parse_filter as parse_dmabuf_filter},
    policy::{FilterPolicy, PolicyInput},
};
use env_logger::Env;
use rustix::event::{PollFd, PollFlags, poll};

const ACCEPT_RESOURCE_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Parser, Debug)]
#[command(name = "d2b-wayland-proxy")]
#[command(about = "Host-side Wayland proxy for d2b graphics VMs")]
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

    /// Override the xdg_toplevel app-id prefix (default: `d2b.<vm>.`).
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

    /// Allow a dmabuf format/modifier in `FORMAT[:MODIFIER]` form (repeatable).
    #[arg(long = "dmabuf-allow", value_name = "FORMAT[:MODIFIER]")]
    dmabuf_allow: Vec<String>,

    /// Deny a dmabuf format/modifier in `FORMAT[:MODIFIER]` form (repeatable).
    #[arg(long = "dmabuf-deny", value_name = "FORMAT[:MODIFIER]")]
    dmabuf_deny: Vec<String>,

    /// Emit a log line for every global filtered from advertisement.
    #[arg(long)]
    log_filtered_globals: bool,

    /// Exact d2b-clipd bridge socket path for this per-user/per-VM proxy.
    #[arg(long = "clipd-bridge-socket", value_name = "PATH")]
    clipd_bridge_socket: Option<PathBuf>,

    /// Root used to derive `/run/d2b/clipd/<uid>/bridge/<vm>/clip.sock`.
    #[arg(
        long = "clipd-bridge-root",
        value_name = "PATH",
        default_value = "/run/d2b/clipd"
    )]
    clipd_bridge_root: PathBuf,

    /// Host Wayland user's numeric uid for derived d2b-clipd bridge paths.
    #[arg(long = "clipd-bridge-user-uid", value_name = "UID")]
    clipd_bridge_user_uid: Option<u32>,

    /// Initial reconnect backoff for the future d2b-clipd bridge.
    #[arg(long = "clipd-bridge-reconnect-initial-ms", default_value_t = 250)]
    clipd_bridge_reconnect_initial_ms: u64,

    /// Maximum reconnect backoff for the future d2b-clipd bridge.
    #[arg(long = "clipd-bridge-reconnect-max-ms", default_value_t = 5000)]
    clipd_bridge_reconnect_max_ms: u64,
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

fn parse_dmabuf_filters(values: &[String]) -> Result<Vec<DmabufFilter>, String> {
    values
        .iter()
        .map(|value| parse_dmabuf_filter(value))
        .collect::<Result<Vec<_>, _>>()
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
            eprintln!("d2b-wayland-proxy: error in --max-version: {e}");
            std::process::exit(1);
        });

    let dmabuf_allow = parse_dmabuf_filters(&args.dmabuf_allow).unwrap_or_else(|e| {
        eprintln!("d2b-wayland-proxy: error in --dmabuf-allow: {e}");
        std::process::exit(1);
    });
    let dmabuf_deny = parse_dmabuf_filters(&args.dmabuf_deny).unwrap_or_else(|e| {
        eprintln!("d2b-wayland-proxy: error in --dmabuf-deny: {e}");
        std::process::exit(1);
    });

    let bridge_config = BridgeConfig::from_parts(
        args.clipd_bridge_socket.clone(),
        &args.clipd_bridge_root,
        args.clipd_bridge_user_uid,
        &args.vm_name,
        BridgeReconnectPolicy {
            initial_delay: Duration::from_millis(args.clipd_bridge_reconnect_initial_ms),
            max_delay: Duration::from_millis(args.clipd_bridge_reconnect_max_ms),
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("d2b-wayland-proxy: error in clipboard bridge configuration: {e}");
        std::process::exit(1);
    });

    let input = PolicyInput {
        vm_name: args.vm_name.clone(),
        app_id_prefix: args.app_id_prefix.clone(),
        title_prefix: args.title_prefix.clone(),
        deny_globals: args.deny_globals.clone(),
        allow_globals: args.allow_globals.clone(),
        max_versions,
        dmabuf_allow,
        dmabuf_deny,
        log_filtered_globals: args.log_filtered_globals,
    };

    // Step 2: resolve policy.
    let policy = FilterPolicy::build(input);

    // Emit advisory warnings to stderr so they appear in the journal.
    for w in &policy.warnings {
        eprintln!("d2b-wayland-proxy: warning: {}", w.message());
    }
    if let Some(path) = &bridge_config.socket_path {
        log::info!(
            "[d2b-wlproxy] vm={} clipboard-bridge={} status=configured",
            args.vm_name,
            path.display()
        );
    } else {
        log::debug!(
            "[d2b-wlproxy] vm={} clipboard-bridge=disabled status=configured",
            args.vm_name
        );
    }

    // Step 3: prove the upstream compositor is reachable before exposing a
    // listen socket. Each accepted client gets its own upstream connection below.
    match build_state(&args.connect) {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "d2b-wayland-proxy: failed to connect to upstream compositor `{}`: {e}",
                args.connect
            );
            std::process::exit(1);
        }
    }

    // Step 4: create the listen socket AFTER successful upstream connect.
    let listen_path = &args.listen;

    // Remove a stale socket if present so restart cycles are idempotent.
    if listen_path.exists()
        && let Err(e) = std::fs::remove_file(listen_path)
    {
        eprintln!(
            "d2b-wayland-proxy: failed to remove stale socket `{}`: {e}",
            listen_path.display()
        );
        std::process::exit(1);
    }

    let listener = match UnixListener::bind(listen_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "d2b-wayland-proxy: failed to bind listen socket `{}`: {e}",
                listen_path.display()
            );
            std::process::exit(1);
        }
    };

    log::info!(
        "[d2b-wlproxy] vm={} listening on {} upstream={}",
        args.vm_name,
        listen_path.display(),
        args.connect
    );

    // Step 5: dispatch loop.
    accept_loop(listener, args.connect, policy, bridge_config);
}

fn accept_loop(
    listener: UnixListener,
    upstream: String,
    policy: FilterPolicy,
    bridge_config: BridgeConfig,
) {
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("d2b-wayland-proxy: failed to set listen socket nonblocking: {e}");
        std::process::exit(1);
    }

    let policy = Rc::new(policy);
    let vm = policy.vm_name.clone();
    let diag = Rc::new(RefCell::new(DiagRateLimiter::new(vm.clone())));
    let clipboard = Rc::new(RefCell::new(VirtualClipboardState::new(
        vm.clone(),
        diag.clone(),
        bridge_config,
    )));
    let state = match build_state(&upstream) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "d2b-wayland-proxy: failed to connect to upstream compositor `{upstream}`: {e}"
            );
            std::process::exit(1);
        }
    };
    state.set_handler(FilterStateHandler::new(
        policy.clone(),
        diag.clone(),
        clipboard.clone(),
    ));

    let mut next_client_id: u64 = 1;
    let mut last_diag_flush = Instant::now();
    let mut listener_backoff_until: Option<Instant> = None;
    while state.is_not_destroyed() {
        let (listener_ready, _state_ready) = {
            let now = Instant::now();
            let listener_in_backoff = listener_backoff_until.is_some_and(|until| until > now);
            if !listener_in_backoff {
                listener_backoff_until = None;
            }
            let diag_timeout = Duration::from_secs(60).saturating_sub(last_diag_flush.elapsed());
            let backoff_timeout = listener_backoff_until
                .map(|until| until.saturating_duration_since(now))
                .unwrap_or(diag_timeout);
            let timeout = diag_timeout
                .min(backoff_timeout)
                .as_millis()
                .min(i32::MAX as u128) as i32;
            let mut poll_fds = [
                PollFd::new(
                    &listener,
                    if listener_in_backoff {
                        PollFlags::empty()
                    } else {
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP
                    },
                ),
                PollFd::new(state.poll_fd(), PollFlags::IN),
            ];
            match poll(&mut poll_fds, timeout) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => {
                    log::warn!("[d2b-wlproxy] vm={vm} poll error: {error}");
                    break;
                }
            }
            (
                !listener_in_backoff && poll_fds[0].revents().contains(PollFlags::IN),
                !poll_fds[1].revents().is_empty(),
            )
        };

        if listener_ready {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let client_id = next_client_id;
                        next_client_id += 1;
                        let fd = Rc::new(stream.into());
                        let client = match state.add_client(&fd) {
                            Ok(c) => c,
                            Err(e) => {
                                let error = bounded_error_detail(error_source_chain(&e));
                                diag.borrow_mut().warn(
                                    "client-accept",
                                    "add-client-failed",
                                    || {
                                        format!(
                                            "[d2b-wlproxy] vm={vm} event=client-accept reason=add-client-failed client={client_id} error={error}"
                                        )
                                    },
                                );
                                continue;
                            }
                        };
                        client.set_handler(FilterClientHandler::with_destructor(
                            vm.clone(),
                            state.create_destructor(),
                        ));
                        install_client_handlers(
                            &client,
                            policy.clone(),
                            diag.clone(),
                            clipboard.clone(),
                        );
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if is_recoverable_accept_error(&e) => {
                        let error = bounded_error_detail(e.to_string());
                        diag.borrow_mut().warn(
                            "client-accept",
                            "recoverable-accept-error",
                            || {
                                format!(
                                    "[d2b-wlproxy] vm={vm} event=client-accept reason=recoverable-accept-error error={error}"
                                )
                            },
                        );
                        if is_resource_exhaustion_accept_error(&e) {
                            listener_backoff_until = Some(Instant::now() + ACCEPT_RESOURCE_BACKOFF);
                        }
                        break;
                    }
                    Err(e) => {
                        eprintln!("d2b-wayland-proxy: accept error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }

        match state.dispatch(Some(Duration::from_secs(0))) {
            Ok(_) => {}
            Err(e) => {
                log::warn!(
                    "[d2b-wlproxy] vm={vm} dispatch error: {}",
                    error_source_chain(&e)
                );
                break;
            }
        }

        if last_diag_flush.elapsed() >= Duration::from_secs(60) {
            diag.borrow_mut().flush_suppressed();
            last_diag_flush = Instant::now();
        }
    }
    state.destroy();
    diag.borrow_mut().flush_suppressed();
}

fn is_recoverable_accept_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Interrupted {
        return true;
    }

    matches!(error.raw_os_error(), Some(libc::ECONNABORTED | libc::EINTR))
        || is_resource_exhaustion_accept_error(error)
}

fn is_resource_exhaustion_accept_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM)
    )
}

/// Renders an error together with its full `source()` chain on one line, e.g.
/// `could not dispatch server events: receiver object 4278190081 does not exist`.
/// `thiserror`'s `Display` only prints the top-level message, so without walking
/// the chain the `#[source]` detail that pinpoints the failing message is lost.
fn error_source_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
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
    fn fd_exhaustion_accept_errors_are_recoverable() {
        for errno in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
            let err = io::Error::from_raw_os_error(errno);
            assert!(is_recoverable_accept_error(&err));
            assert!(is_resource_exhaustion_accept_error(&err));
        }
    }

    #[test]
    fn permission_denied_accept_is_fatal() {
        let err = io::Error::from_raw_os_error(libc::EACCES);
        assert!(!is_recoverable_accept_error(&err));
    }

    #[test]
    fn error_source_chain_includes_nested_causes() {
        use std::error::Error;
        use std::fmt;

        #[derive(Debug)]
        struct Inner;
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "receiver object 42 does not exist")
            }
        }
        impl Error for Inner {}

        #[derive(Debug)]
        struct Outer(Inner);
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "could not dispatch server events")
            }
        }
        impl Error for Outer {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(
            error_source_chain(&Outer(Inner)),
            "could not dispatch server events: receiver object 42 does not exist"
        );
    }
}
