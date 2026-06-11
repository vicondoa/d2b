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
    env, io,
    io::IoSlice,
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use env_logger::Env;
use nix::{
    cmsg_space,
    sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags},
    unistd::close,
};
use nixling_wayland_filter::filter::{
    build_state, install_client_handlers, FilterClientHandler, FilterStateHandler,
};
use nixling_wayland_filter::{
    diag::DiagRateLimiter,
    dmabuf::{parse_filter as parse_dmabuf_filter, DmabufFilter},
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

    /// Allow a dmabuf format/modifier in `FORMAT[:MODIFIER]` form (repeatable).
    #[arg(long = "dmabuf-allow", value_name = "FORMAT[:MODIFIER]")]
    dmabuf_allow: Vec<String>,

    /// Deny a dmabuf format/modifier in `FORMAT[:MODIFIER]` form (repeatable).
    #[arg(long = "dmabuf-deny", value_name = "FORMAT[:MODIFIER]")]
    dmabuf_deny: Vec<String>,

    /// Emit a log line for every global filtered from advertisement.
    #[arg(long)]
    log_filtered_globals: bool,

    /// Diagnostic mode: bypass wl-proxy and relay raw Wayland bytes/fds unchanged.
    ///
    /// This disables global filtering and app-id/title rewriting. It exists only
    /// to compare the byte stream crosvm sees against the normal filtered path.
    #[arg(long, hide = true)]
    raw_relay: bool,
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
            eprintln!("nixling-wayland-filter: error in --max-version: {e}");
            std::process::exit(1);
        });

    let dmabuf_allow = parse_dmabuf_filters(&args.dmabuf_allow).unwrap_or_else(|e| {
        eprintln!("nixling-wayland-filter: error in --dmabuf-allow: {e}");
        std::process::exit(1);
    });
    let dmabuf_deny = parse_dmabuf_filters(&args.dmabuf_deny).unwrap_or_else(|e| {
        eprintln!("nixling-wayland-filter: error in --dmabuf-deny: {e}");
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
        eprintln!("nixling-wayland-filter: warning: {}", w.message());
    }

    // Step 3: prove the upstream compositor is reachable before exposing a
    // listen socket. Each accepted client gets its own upstream connection below.
    if args.raw_relay {
        if let Err(e) = UnixStream::connect(&args.connect) {
            eprintln!(
                "nixling-wayland-filter: failed to connect raw relay upstream `{}`: {e}",
                args.connect
            );
            std::process::exit(1);
        }
    } else {
        match build_state(&args.connect) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "nixling-wayland-filter: failed to connect to upstream compositor `{}`: {e}",
                    args.connect
                );
                std::process::exit(1);
            }
        }
    }

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

    log::info!(
        "[nixling-wlproxy] vm={} listening on {} upstream={}",
        args.vm_name,
        listen_path.display(),
        args.connect
    );

    // Step 5: dispatch loop.
    accept_loop(listener, args.connect, policy, args.raw_relay);
}

fn accept_loop(listener: UnixListener, upstream: String, policy: FilterPolicy, raw_relay: bool) {
    let vm = policy.vm_name.clone();
    let mut next_client_id: u64 = 1;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let client_id = next_client_id;
                next_client_id += 1;
                let upstream = upstream.clone();
                let policy = policy.clone();
                let vm = vm.clone();
                let name = format!("nixling-wlproxy-{vm}-{client_id}");
                let thread_vm = vm.clone();
                let spawn = thread::Builder::new().name(name).spawn(move || {
                    if raw_relay {
                        run_raw_relay_client(client_id, stream, &upstream, &thread_vm);
                    } else {
                        run_client(client_id, stream.into(), &upstream, policy);
                    }
                });
                if let Err(e) = spawn {
                    log::warn!("[nixling-wlproxy] vm={vm} failed to spawn client thread: {e}");
                }
            }
            Err(e) if is_recoverable_accept_error(&e) => {
                log::warn!("[nixling-wlproxy] vm={vm} recoverable accept error: {e}");
            }
            Err(e) => {
                eprintln!("nixling-wayland-filter: accept error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn run_client(client_id: u64, fd: OwnedFd, upstream: &str, policy: FilterPolicy) {
    let policy = Rc::new(policy);
    let vm = policy.vm_name.clone();
    let diag = Rc::new(RefCell::new(DiagRateLimiter::new(vm.clone())));

    let state = match build_state(upstream) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[nixling-wlproxy] vm={vm} client={client_id} upstream connect failed: {e}");
            return;
        }
    };
    state.set_handler(FilterStateHandler::new(policy.clone(), diag.clone()));

    let fd = Rc::new(fd);
    let client = match state.add_client(&fd) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[nixling-wlproxy] vm={vm} client={client_id} failed to add client: {e}");
            return;
        }
    };
    client.set_handler(FilterClientHandler::with_destructor(
        vm.clone(),
        state.create_destructor(),
    ));
    install_client_handlers(&client, policy, diag.clone());

    let mut last_diag_flush = Instant::now();
    while state.is_not_destroyed() {
        match state.dispatch(Some(Duration::from_millis(10))) {
            Ok(_) => {}
            Err(e) => {
                log::warn!("[nixling-wlproxy] vm={vm} client={client_id} dispatch error: {e}");
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

fn run_raw_relay_client(client_id: u64, client: UnixStream, upstream: &str, vm: &str) {
    let server = match UnixStream::connect(upstream) {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!(
                "[nixling-raw-wlrelay] vm={vm} client={client_id} upstream connect failed: {e}"
            );
            return;
        }
    };

    let client_to_server_read = match client.try_clone() {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("[nixling-raw-wlrelay] vm={vm} client={client_id} client clone failed: {e}");
            return;
        }
    };
    let server_to_client_read = match server.try_clone() {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("[nixling-raw-wlrelay] vm={vm} client={client_id} server clone failed: {e}");
            return;
        }
    };

    let vm = Arc::new(vm.to_owned());
    log::info!("[nixling-raw-wlrelay] vm={vm} client={client_id} raw relay connected");

    let vm_for_c2s = vm.clone();
    let c2s = thread::spawn(move || {
        relay_raw_direction(
            &vm_for_c2s,
            client_id,
            "client->raw",
            "raw->server",
            client_to_server_read,
            server,
        )
    });
    let s2c = thread::spawn(move || {
        relay_raw_direction(
            &vm,
            client_id,
            "server->raw",
            "raw->client",
            server_to_client_read,
            client,
        )
    });

    let _ = c2s.join();
    let _ = s2c.join();
}

fn relay_raw_direction(
    vm: &str,
    client_id: u64,
    recv_label: &str,
    send_label: &str,
    read: UnixStream,
    write: UnixStream,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let mut fds = Vec::<RawFd>::new();
        let len = match recv_raw(&read, &mut buf, &mut fds) {
            Ok(0) => break,
            Ok(len) => len,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::warn!(
                    "[nixling-raw-wlrelay] vm={vm} client={client_id} {recv_label} recv failed: {e}"
                );
                break;
            }
        };

        log_raw_hexdump(
            vm,
            client_id,
            "recv",
            recv_label,
            read.as_raw_fd(),
            &buf[..len],
            fds.len(),
        );
        if let Err(e) = send_raw(&write, &buf[..len], &fds) {
            log::warn!(
                "[nixling-raw-wlrelay] vm={vm} client={client_id} {send_label} send failed: {e}"
            );
            close_fds(&fds);
            break;
        }
        log_raw_hexdump(
            vm,
            client_id,
            "send",
            send_label,
            write.as_raw_fd(),
            &buf[..len],
            fds.len(),
        );
        close_fds(&fds);
    }
}

fn recv_raw(stream: &UnixStream, buf: &mut [u8], fds: &mut Vec<RawFd>) -> io::Result<usize> {
    let mut iov = [io::IoSliceMut::new(buf)];
    let mut cmsg = cmsg_space!([RawFd; 253]);
    let msg = recvmsg::<()>(
        stream.as_raw_fd(),
        &mut iov,
        Some(&mut cmsg),
        MsgFlags::empty(),
    )
    .map_err(io::Error::from)?;
    for cmsg in msg.cmsgs().map_err(io::Error::from)? {
        if let ControlMessageOwned::ScmRights(rights) = cmsg {
            fds.extend(rights);
        }
    }
    Ok(msg.bytes)
}

fn send_raw(stream: &UnixStream, buf: &[u8], fds: &[RawFd]) -> io::Result<()> {
    let iov = [IoSlice::new(buf)];
    let res = if fds.is_empty() {
        sendmsg::<()>(stream.as_raw_fd(), &iov, &[], MsgFlags::empty(), None)
    } else {
        let cmsgs = [ControlMessage::ScmRights(fds)];
        sendmsg::<()>(stream.as_raw_fd(), &iov, &cmsgs, MsgFlags::empty(), None)
    };
    let written = res.map_err(io::Error::from)?;
    if written != buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("partial sendmsg wrote {written}/{} bytes", buf.len()),
        ));
    }
    Ok(())
}

fn close_fds(fds: &[RawFd]) {
    for fd in fds {
        let _ = close(*fd);
    }
}

fn raw_hexdump_enabled() -> bool {
    std::env::var("WL_PROXY_HEXDUMP").as_deref() == Ok("1")
}

fn raw_hexdump_limit() -> usize {
    env::var("WL_PROXY_HEXDUMP_LIMIT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(8192)
}

fn log_raw_hexdump(
    vm: &str,
    client_id: u64,
    kind: &str,
    label: &str,
    socket: RawFd,
    bytes: &[u8],
    fd_count: usize,
) {
    if !raw_hexdump_enabled() {
        return;
    }
    let limit = raw_hexdump_limit();
    let shown = bytes.len().min(limit);
    let mut hex = String::with_capacity(shown.saturating_mul(3));
    for (idx, byte) in bytes.iter().take(shown).enumerate() {
        if idx > 0 {
            hex.push(' ');
        }
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    let truncated = bytes.len().saturating_sub(shown);
    eprintln!(
        "[wl-raw-relay-hexdump] vm={vm} client={client_id} kind={kind} label={label} socket={socket} bytes={} fds={fd_count} shown={shown} truncated={truncated} hex={hex}",
        bytes.len(),
    );
}

fn is_recoverable_accept_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Interrupted {
        return true;
    }

    matches!(error.raw_os_error(), Some(libc::ECONNABORTED | libc::EINTR))
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
