//! Provider-neutral host-side Wayland proxy for d2b workloads.
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
    ffi::OsString,
    io,
    os::unix::net::UnixListener,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use clap::Parser;
use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use d2b_wayland_proxy::filter::{
    FilterStateHandler, VirtualClipboardState, build_state, install_client_handlers,
};
use d2b_wayland_proxy::{
    bridge::{BridgeConfig, BridgeReconnectPolicy},
    decoration::{BorderConfig, Color, DecorationManager, LabelPosition, sanitize_label},
    diag::{DiagRateLimiter, bounded_error_detail},
    dmabuf::{DmabufFilter, parse_filter as parse_dmabuf_filter},
    identity::ProxyIdentity,
    policy::{FilterPolicy, PolicyInput},
    readiness::{ProxyReadinessFailure, ProxyReadinessStage, ReadinessReporter},
    terminal::{
        TerminalChild, TerminalRuntime, child_exit_code, chmod_socket_strict,
        unlink_stale_socket_path,
    },
};
use env_logger::Env;
use rustix::event::{PollFd, PollFlags, poll};
use smallvec::{SmallVec, smallvec};

const ACCEPT_RESOURCE_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Parser, Debug)]
#[command(name = "d2b-wayland-proxy")]
#[command(about = "Provider-neutral host-side Wayland proxy for d2b workloads")]
struct Args {
    /// Path of the Unix socket to create and accept client connections on.
    #[arg(long)]
    listen: Option<PathBuf>,

    /// Path of the upstream host compositor socket to connect to.
    #[arg(long)]
    connect: Option<String>,

    /// Legacy VM name retained during compatibility migration.
    #[arg(long, value_name = "VM")]
    vm_name: Option<String>,

    /// Canonical workload target, e.g. `tools.host.d2b`.
    #[arg(long, value_name = "TARGET")]
    target: Option<String>,

    /// Provider kind for --target.
    #[arg(long, value_name = "KIND")]
    provider_kind: Option<String>,

    /// Override the xdg_toplevel app-id prefix (default: `d2b.<vm>.`).
    #[arg(long)]
    app_id_prefix: Option<String>,

    /// Canonical d2b realm target asserted by host metadata.
    #[arg(long = "realm-target", value_name = "TARGET")]
    realm_target: Option<String>,

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

    /// Exact d2b-clipd bridge socket path for this per-user/per-workload proxy.
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

    /// Enable proxy-owned workload identity rails.
    #[arg(long = "border-enable", default_value_t = false)]
    border_enable: bool,

    /// Rail color when a workload window is active.
    #[arg(
        long = "border-color-active",
        value_name = "#rrggbb",
        default_value = "#3caaff"
    )]
    border_color_active: Color,

    /// Rail color when a workload window is inactive.
    #[arg(
        long = "border-color-inactive",
        value_name = "#rrggbb",
        default_value = "#5f5f5f"
    )]
    border_color_inactive: Color,

    /// Rail color reserved for urgent workload windows.
    #[arg(
        long = "border-color-urgent",
        value_name = "#rrggbb",
        default_value = "#ff5656"
    )]
    border_color_urgent: Color,

    /// Deprecated legacy border thickness; wrapper rails use a fixed width.
    #[arg(long = "border-thickness", value_parser = parse_positive_u32, default_value_t = d2b_wayland_proxy::decoration::DEFAULT_BORDER_THICKNESS)]
    border_thickness: u32,

    /// Optional text rendered into the wrapper rail.
    #[arg(long = "border-label", value_name = "TEXT")]
    border_label: Option<String>,

    /// Deprecated legacy label position; wrapper rails always use a vertical label.
    #[arg(
        long = "border-label-position",
        value_name = "top-left|top-center",
        default_value = "top-left"
    )]
    border_label_position: LabelPosition,

    /// Launch a foreground WezTerm child through a randomized single-use proxy socket.
    #[arg(long = "host-terminal")]
    host_terminal: bool,

    /// Terminal program to launch in --host-terminal mode.
    #[arg(long = "terminal-program", default_value = "wezterm")]
    terminal_program: OsString,

    /// Arguments passed to the terminal program after `--` in --host-terminal mode.
    #[arg(last = true, allow_hyphen_values = true)]
    terminal_args: Vec<OsString>,

    /// Connected Unix stream used for typed readiness events.
    #[arg(long = "readiness-socket", value_name = "PATH")]
    readiness_socket: Option<PathBuf>,

    /// Fail if the first proxied client does not connect before this deadline.
    #[arg(long = "first-client-timeout-ms", value_parser = parse_positive_u64)]
    first_client_timeout_ms: Option<u64>,
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

fn parse_positive_u32(value: &str) -> Result<u32, String> {
    let parsed: u32 = value
        .parse()
        .map_err(|_| format!("expected positive u32, got `{value}`"))?;
    if parsed == 0 {
        Err("border thickness must be positive".to_owned())
    } else {
        Ok(parsed)
    }
}

fn parse_positive_u64(value: &str) -> Result<u64, String> {
    let parsed: u64 = value
        .parse()
        .map_err(|_| format!("expected positive u64, got `{value}`"))?;
    if parsed == 0 {
        Err("timeout must be positive".to_owned())
    } else {
        Ok(parsed)
    }
}

fn parse_provider_kind(value: &str) -> Result<WorkloadProviderKind, String> {
    match value {
        "local-vm" => Ok(WorkloadProviderKind::LocalVm),
        "qemu-media" => Ok(WorkloadProviderKind::QemuMedia),
        "provider-managed" => Ok(WorkloadProviderKind::ProviderManaged),
        "unsafe-local" => Ok(WorkloadProviderKind::UnsafeLocal),
        _ => Err("unsupported provider kind".to_owned()),
    }
}

fn resolve_identity(args: &Args) -> Result<ProxyIdentity, String> {
    if let Some(raw_target) = &args.target {
        let target = WorkloadTarget::parse(raw_target)
            .map_err(|_| "--target must be a canonical workload target".to_owned())?;
        let provider = args
            .provider_kind
            .as_deref()
            .ok_or_else(|| "--provider-kind is required with --target".to_owned())
            .and_then(parse_provider_kind)?;
        return match &args.vm_name {
            Some(vm_name) => ProxyIdentity::legacy_vm(vm_name, target, provider)
                .map_err(|error| error.to_string()),
            None => Ok(ProxyIdentity::canonical(target, provider)),
        };
    }

    if args.provider_kind.is_some() {
        return Err("--provider-kind requires --target".to_owned());
    }
    let vm_name = args
        .vm_name
        .as_deref()
        .ok_or_else(|| "either --target or compatibility --vm-name is required".to_owned())?;
    let target = match args.realm_target.as_deref() {
        Some(target) => WorkloadTarget::parse(target)
            .map_err(|_| "--realm-target must be a canonical workload target".to_owned())?,
        None => WorkloadTarget::parse(&format!("{vm_name}.local.d2b"))
            .map_err(|_| "--vm-name cannot form a canonical workload target".to_owned())?,
    };
    ProxyIdentity::legacy_vm(vm_name, target, WorkloadProviderKind::LocalVm)
        .map_err(|error| error.to_string())
}

fn configured_first_client_timeout(args: &Args, identity: &ProxyIdentity) -> Option<Duration> {
    args.first_client_timeout_ms
        .map(Duration::from_millis)
        .or_else(|| {
            (identity.provider_kind() == WorkloadProviderKind::UnsafeLocal)
                .then_some(Duration::from_secs(10))
        })
}

fn validate_execution_mode(args: &Args, identity: &ProxyIdentity) -> Result<(), String> {
    if identity.provider_kind() == WorkloadProviderKind::UnsafeLocal && args.host_terminal {
        return Err(
            "--host-terminal is compatibility-only; unsafe-local apps must be launched by the authenticated helper"
                .to_owned(),
        );
    }
    Ok(())
}

fn bound_poll_timeout_to_deadline(base_ms: i32, deadline: Instant, now: Instant) -> i32 {
    base_ms.min(
        deadline
            .saturating_duration_since(now)
            .as_millis()
            .min(i32::MAX as u128) as i32,
    )
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    let identity = resolve_identity(&args).unwrap_or_else(|error| {
        eprintln!("d2b-wayland-proxy: {error}");
        std::process::exit(1);
    });
    if let Err(error) = validate_execution_mode(&args, &identity) {
        eprintln!("d2b-wayland-proxy: {error}");
        std::process::exit(1);
    }
    let mut readiness = match &args.readiness_socket {
        Some(path) => ReadinessReporter::connect(identity.clone(), path).unwrap_or_else(|_| {
            eprintln!("d2b-wayland-proxy: typed readiness channel unavailable");
            std::process::exit(1);
        }),
        None => ReadinessReporter::disabled(identity.clone()),
    };

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

    let bridge_config = BridgeConfig::from_identity_parts(
        args.clipd_bridge_socket.clone(),
        &args.clipd_bridge_root,
        args.clipd_bridge_user_uid,
        &identity,
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
        identity: Some(identity.clone()),
        vm_name: identity.log_label(),
        app_id_prefix: args.app_id_prefix.clone(),
        realm_target: Some(identity.canonical_target()),
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
            "[d2b-wlproxy] target={} provider={} clipboard-bridge={} status=configured",
            identity.canonical_target(),
            identity.provider_kind_label(),
            path.display()
        );
    } else {
        log::debug!(
            "[d2b-wlproxy] target={} provider={} clipboard-bridge=disabled status=configured",
            identity.canonical_target(),
            identity.provider_kind_label()
        );
    }

    let border_label = args
        .border_label
        .as_deref()
        .and_then(sanitize_label)
        .or_else(|| {
            (identity.provider_kind() == WorkloadProviderKind::UnsafeLocal)
                .then(|| identity.default_warning_label())
                .and_then(|label| sanitize_label(&label))
        });
    let border_config = BorderConfig {
        enabled: args.border_enable,
        active: args.border_color_active,
        inactive: args.border_color_inactive,
        urgent: args.border_color_urgent,
        thickness: args.border_thickness,
        label: border_label,
        label_position: args.border_label_position,
    };
    if border_config.enabled() {
        log::info!(
            "[d2b-wlproxy] target={} provider={} wrapper-rail=enabled label={}",
            identity.canonical_target(),
            identity.provider_kind_label(),
            border_config
                .label
                .as_ref()
                .map_or("disabled", |_| "enabled")
        );
    }

    let upstream = resolve_upstream(&args, &identity).unwrap_or_else(|e| {
        let _ = readiness.failed(
            ProxyReadinessStage::Upstream,
            ProxyReadinessFailure::UpstreamUnavailable,
        );
        eprintln!("d2b-wayland-proxy: {e}");
        std::process::exit(1);
    });

    let terminal_runtime = if args.host_terminal {
        Some(
            TerminalRuntime::prepare(&identity.bridge_component()).unwrap_or_else(|e| {
                eprintln!("d2b-wayland-proxy: failed to prepare host-terminal runtime: {e}");
                std::process::exit(1);
            }),
        )
    } else {
        None
    };
    let listen_path = resolve_listen_path(&args, terminal_runtime.as_ref()).unwrap_or_else(|e| {
        eprintln!("d2b-wayland-proxy: {e}");
        std::process::exit(1);
    });

    // Step 3: prove the upstream compositor is reachable before exposing a
    // listen socket. Each accepted client gets its own upstream connection below.
    match build_state(&upstream) {
        Ok(_) => {}
        Err(e) => {
            let _ = readiness.failed(
                ProxyReadinessStage::Upstream,
                ProxyReadinessFailure::UpstreamUnavailable,
            );
            eprintln!(
                "d2b-wayland-proxy: failed to connect to upstream compositor `{}`: {e}",
                upstream
            );
            std::process::exit(1);
        }
    }
    if readiness.ready(ProxyReadinessStage::Upstream).is_err() {
        eprintln!("d2b-wayland-proxy: typed readiness channel unavailable");
        std::process::exit(1);
    }

    // Step 4: create the listen socket AFTER successful upstream connect.
    // Remove a stale socket if present so restart cycles are idempotent.
    if listen_path.exists() {
        let stale_result = if terminal_runtime.is_some() {
            unlink_stale_socket_path(&listen_path)
        } else {
            std::fs::remove_file(&listen_path)
        };
        if let Err(e) = stale_result {
            let _ = readiness.failed(
                ProxyReadinessStage::Listener,
                ProxyReadinessFailure::ListenerUnavailable,
            );
            eprintln!(
                "d2b-wayland-proxy: failed to remove stale socket `{}`: {e}",
                listen_path.display()
            );
            std::process::exit(1);
        }
    }

    let listener = match UnixListener::bind(&listen_path) {
        Ok(l) => l,
        Err(e) => {
            let _ = readiness.failed(
                ProxyReadinessStage::Listener,
                ProxyReadinessFailure::ListenerUnavailable,
            );
            eprintln!(
                "d2b-wayland-proxy: failed to bind listen socket `{}`: {e}",
                listen_path.display()
            );
            std::process::exit(1);
        }
    };
    if terminal_runtime.is_some()
        && let Err(e) = chmod_socket_strict(&listen_path)
    {
        let _ = readiness.failed(
            ProxyReadinessStage::Listener,
            ProxyReadinessFailure::ListenerUnavailable,
        );
        eprintln!(
            "d2b-wayland-proxy: failed to secure listen socket `{}`: {e}",
            listen_path.display()
        );
        std::process::exit(1);
    }
    if readiness.ready(ProxyReadinessStage::Listener).is_err() {
        eprintln!("d2b-wayland-proxy: typed readiness channel unavailable");
        std::process::exit(1);
    }

    log::info!(
        "[d2b-wlproxy] target={} provider={} listening on {} upstream={}",
        identity.canonical_target(),
        identity.provider_kind_label(),
        listen_path.display(),
        upstream
    );

    let mut terminal_child = terminal_runtime.as_ref().map(|runtime| {
        TerminalChild::spawn(&args.terminal_program, &args.terminal_args, runtime).unwrap_or_else(
            |e| {
                let _ = readiness.failed(
                    ProxyReadinessStage::FirstClient,
                    ProxyReadinessFailure::ClientRejected,
                );
                eprintln!("d2b-wayland-proxy: failed to launch host terminal child: {e}");
                std::process::exit(1);
            },
        )
    });
    if terminal_child.is_some() {
        log::info!(
            "[d2b-wlproxy] target={} event=host-terminal-launched mux=per-target",
            identity.canonical_target()
        );
    }

    let first_client_timeout = configured_first_client_timeout(&args, &identity);

    // Step 5: dispatch loop.
    let exit_code = accept_loop(
        listener,
        upstream,
        policy,
        bridge_config,
        border_config,
        terminal_child.as_mut(),
        AcceptLoopControl {
            readiness,
            first_client_timeout,
        },
    );
    if let Some(child) = terminal_child.as_mut()
        && exit_code != 0
    {
        child.terminate();
    }
    drop(terminal_runtime);
    std::process::exit(exit_code);
}

fn resolve_upstream(args: &Args, identity: &ProxyIdentity) -> Result<String, String> {
    if let Some(connect) = &args.connect {
        return Ok(connect.clone());
    }
    if args.host_terminal && identity.provider_kind() != WorkloadProviderKind::UnsafeLocal {
        return std::env::var("WAYLAND_DISPLAY").map_err(|_| {
            "WAYLAND_DISPLAY is required by compatibility host-terminal mode".to_owned()
        });
    }
    Err("--connect is required; d2b never falls back to a direct compositor route".to_owned())
}

fn resolve_listen_path(
    args: &Args,
    terminal_runtime: Option<&TerminalRuntime>,
) -> Result<PathBuf, String> {
    match (&args.listen, terminal_runtime) {
        (Some(path), None) => Ok(path.clone()),
        (Some(_), Some(_)) => {
            Err("--listen cannot be combined with randomized --host-terminal mode".to_owned())
        }
        (None, Some(runtime)) => Ok(runtime.listen_socket().to_owned()),
        (None, None) => Err("--listen is required unless --host-terminal is used".to_owned()),
    }
}

struct AcceptLoopControl {
    readiness: ReadinessReporter,
    first_client_timeout: Option<Duration>,
}

fn accept_loop(
    listener: UnixListener,
    upstream: String,
    policy: FilterPolicy,
    bridge_config: BridgeConfig,
    border_config: BorderConfig,
    mut terminal_child: Option<&mut TerminalChild>,
    control: AcceptLoopControl,
) -> i32 {
    let AcceptLoopControl {
        mut readiness,
        first_client_timeout,
    } = control;
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("d2b-wayland-proxy: failed to set listen socket nonblocking: {e}");
        std::process::exit(1);
    }

    let policy = Rc::new(policy);
    let vm = policy.vm_name.clone();
    let diag = Rc::new(RefCell::new(DiagRateLimiter::new(vm.clone())));
    let clipboard = Rc::new(RefCell::new(VirtualClipboardState::new(
        policy.identity.clone(),
        diag.clone(),
        bridge_config,
    )));
    let decoration = border_config.enabled().then(|| {
        Rc::new(RefCell::new(DecorationManager::new(
            border_config,
            diag.clone(),
        )))
    });
    let state = match build_state(&upstream) {
        Ok(s) => s,
        Err(e) => {
            let _ = readiness.failed(
                ProxyReadinessStage::Upstream,
                ProxyReadinessFailure::UpstreamUnavailable,
            );
            eprintln!(
                "d2b-wayland-proxy: failed to connect to upstream compositor `{upstream}`: {e}"
            );
            return 1;
        }
    };
    state.set_handler(FilterStateHandler::new(
        policy.clone(),
        diag.clone(),
        clipboard.clone(),
        decoration.clone(),
    ));
    VirtualClipboardState::drive_bridge_io(&clipboard, false);

    let mut next_client_id: u64 = 1;
    let mut last_diag_flush = Instant::now();
    let mut listener_backoff_until: Option<Instant> = None;
    let mut first_client_deadline = first_client_timeout.map(|timeout| Instant::now() + timeout);
    while state.is_not_destroyed() {
        if first_client_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = readiness.failed(
                ProxyReadinessStage::FirstClient,
                ProxyReadinessFailure::FirstClientTimeout,
            );
            if let Some(child) = terminal_child.as_mut() {
                child.terminate();
            }
            state.destroy();
            diag.borrow_mut().flush_suppressed();
            return 70;
        }
        if let Some(child) = terminal_child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.destroy();
                    diag.borrow_mut().flush_suppressed();
                    return child_exit_code(status);
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!("[d2b-wlproxy] target={vm} event=host-terminal-wait error={error}");
                    break;
                }
            }
        }
        let (listener_ready, _state_ready, bridge_ready) = {
            let now = Instant::now();
            let listener_in_backoff = accept_backoff_active(listener_backoff_until, now);
            if !listener_in_backoff {
                listener_backoff_until = None;
            }
            let diag_timeout = Duration::from_secs(60).saturating_sub(last_diag_flush.elapsed());
            let clipboard_ref = clipboard.borrow();
            let timeout = accept_poll_timeout_ms(
                diag_timeout,
                listener_backoff_until,
                clipboard_ref.bridge_retry_deadline(),
                now,
                terminal_child.is_some(),
            );
            let timeout = first_client_deadline
                .map(|deadline| bound_poll_timeout_to_deadline(timeout, deadline, now))
                .unwrap_or(timeout);
            let mut poll_fds: SmallVec<[PollFd<'_>; 3]> = smallvec![
                PollFd::new(&listener, listener_accept_poll_flags(listener_in_backoff)),
                PollFd::new(state.poll_fd(), PollFlags::IN),
            ];
            let bridge_poll_index = poll_fds.len();
            if let Some((bridge, flags)) = clipboard_ref.bridge_poll_stream_and_flags() {
                poll_fds.push(PollFd::new(bridge, flags));
            }
            match poll(&mut poll_fds, timeout) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => {
                    log::warn!("[d2b-wlproxy] target={vm} poll error: {error}");
                    break;
                }
            }
            (
                !listener_in_backoff && poll_fds[0].revents().contains(PollFlags::IN),
                !poll_fds[1].revents().is_empty(),
                poll_fds
                    .get(bridge_poll_index)
                    .is_some_and(|fd| !fd.revents().is_empty()),
            )
        };

        if listener_ready {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(true) {
                            let error = bounded_error_detail(error.to_string());
                            diag.borrow_mut().warn(
                                "client-accept",
                                "client-nonblocking-failed",
                                || {
                                    format!(
                                        "[d2b-wlproxy] target={vm} event=client-accept reason=client-nonblocking-failed error={error}"
                                    )
                                },
                            );
                            continue;
                        }
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
                                            "[d2b-wlproxy] target={vm} event=client-accept reason=add-client-failed client={client_id} error={error}"
                                        )
                                    },
                                );
                                continue;
                            }
                        };
                        install_client_handlers(
                            &client,
                            policy.clone(),
                            diag.clone(),
                            clipboard.clone(),
                            decoration.clone(),
                        );
                        if first_client_deadline.take().is_some()
                            && readiness.ready(ProxyReadinessStage::FirstClient).is_err()
                        {
                            if let Some(child) = terminal_child.as_mut() {
                                child.terminate();
                            }
                            state.destroy();
                            diag.borrow_mut().flush_suppressed();
                            return 70;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if is_recoverable_accept_error(&e) => {
                        let error = bounded_error_detail(e.to_string());
                        diag.borrow_mut().warn(
                            "client-accept",
                            "recoverable-accept-error",
                            || {
                                format!(
                                    "[d2b-wlproxy] target={vm} event=client-accept reason=recoverable-accept-error error={error}"
                                )
                            },
                        );
                        if is_resource_exhaustion_accept_error(&e) {
                            listener_backoff_until = Some(Instant::now() + ACCEPT_RESOURCE_BACKOFF);
                        }
                        break;
                    }
                    Err(e) => {
                        let _ = readiness.failed(
                            ProxyReadinessStage::FirstClient,
                            ProxyReadinessFailure::ClientRejected,
                        );
                        eprintln!("d2b-wayland-proxy: accept error: {e}");
                        state.destroy();
                        diag.borrow_mut().flush_suppressed();
                        return 1;
                    }
                }
            }
        }

        match state.dispatch(Some(Duration::from_secs(0))) {
            Ok(_) => {}
            Err(e) => {
                log::warn!(
                    "[d2b-wlproxy] target={vm} dispatch error: {}",
                    error_source_chain(&e)
                );
                break;
            }
        }
        VirtualClipboardState::drive_bridge_io(&clipboard, bridge_ready);
        if bridge_ready {
            VirtualClipboardState::drain_bridge_messages(&clipboard);
        }

        if last_diag_flush.elapsed() >= Duration::from_secs(60) {
            diag.borrow_mut().flush_suppressed();
            last_diag_flush = Instant::now();
        }
    }
    state.destroy();
    diag.borrow_mut().flush_suppressed();
    0
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

fn accept_backoff_active(backoff_until: Option<Instant>, now: Instant) -> bool {
    backoff_until.is_some_and(|until| until > now)
}

fn listener_accept_poll_flags(in_backoff: bool) -> PollFlags {
    if in_backoff {
        PollFlags::empty()
    } else {
        PollFlags::IN | PollFlags::ERR | PollFlags::HUP
    }
}

fn accept_poll_timeout_ms(
    diag_timeout: Duration,
    listener_backoff_until: Option<Instant>,
    bridge_retry_until: Option<Instant>,
    now: Instant,
    watching_child: bool,
) -> i32 {
    let backoff_timeout = listener_backoff_until
        .map(|until| until.saturating_duration_since(now))
        .unwrap_or(diag_timeout);
    let bridge_timeout = bridge_retry_until
        .map(|until| until.saturating_duration_since(now))
        .unwrap_or(diag_timeout);
    let child_timeout = if watching_child {
        Duration::from_millis(100)
    } else {
        diag_timeout
    };
    diag_timeout
        .min(backoff_timeout)
        .min(bridge_timeout)
        .min(child_timeout)
        .as_millis()
        .min(i32::MAX as u128) as i32
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
    fn border_cli_defaults_keep_decorations_disabled() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--listen",
            "target/test.sock",
            "--connect",
            "wayland-1",
            "--vm-name",
            "work",
        ])
        .expect("parse args");

        assert!(!args.border_enable);
        assert_eq!(
            args.border_thickness,
            d2b_wayland_proxy::decoration::DEFAULT_BORDER_THICKNESS
        );
        assert!(args.border_label.is_none());
    }

    #[test]
    fn border_cli_parses_colors_and_legacy_shape_flags() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--listen",
            "target/test.sock",
            "--connect",
            "wayland-1",
            "--vm-name",
            "work",
            "--realm-target",
            "work.local.d2b",
            "--border-enable",
            "--border-color-active",
            "#112233",
            "--border-color-inactive",
            "#445566",
            "--border-color-urgent",
            "#778899",
            "--border-thickness",
            "9",
            "--border-label",
            "work vm",
            "--border-label-position",
            "top-center",
        ])
        .expect("parse args");

        assert!(args.border_enable);
        assert_eq!(args.border_color_active, Color::rgb(0x11, 0x22, 0x33));
        assert_eq!(args.border_color_inactive, Color::rgb(0x44, 0x55, 0x66));
        assert_eq!(args.border_color_urgent, Color::rgb(0x77, 0x88, 0x99));
        assert_eq!(args.border_thickness, 9);
        assert_eq!(args.border_label.as_deref(), Some("work vm"));
        assert_eq!(args.border_label_position, LabelPosition::TopCenter);
        assert_eq!(args.realm_target.as_deref(), Some("work.local.d2b"));
    }

    #[test]
    fn host_terminal_cli_does_not_require_static_listen_or_connect() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--host-terminal",
            "--vm-name",
            "work",
            "--",
            "start",
            "--always-new-process",
        ])
        .expect("parse args");

        assert!(args.host_terminal);
        assert_eq!(args.vm_name.as_deref(), Some("work"));
        assert!(args.listen.is_none());
        assert!(args.connect.is_none());
        assert_eq!(
            args.terminal_args,
            vec![
                OsString::from("start"),
                OsString::from("--always-new-process")
            ]
        );
    }

    #[test]
    fn regular_proxy_still_requires_explicit_listen_and_connect() {
        let args = Args::try_parse_from(["d2b-wayland-proxy", "--vm-name", "work"])
            .expect("deferred validation keeps clap errors friendly");
        let identity = resolve_identity(&args).expect("legacy identity");

        assert!(resolve_listen_path(&args, None).is_err());
        assert!(resolve_upstream(&args, &identity).is_err());
    }

    #[test]
    fn canonical_unsafe_local_cli_requires_explicit_upstream_and_has_no_vm_identity() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--target",
            "browser.host.d2b",
            "--provider-kind",
            "unsafe-local",
            "--listen",
            "target/test.sock",
        ])
        .expect("parse canonical args");
        let identity = resolve_identity(&args).expect("canonical identity");

        assert_eq!(identity.canonical_target(), "browser.host.d2b");
        assert_eq!(identity.provider_kind(), WorkloadProviderKind::UnsafeLocal);
        assert!(identity.legacy_vm_name().is_none());
        assert!(
            resolve_upstream(&args, &identity)
                .expect_err("unsafe-local never guesses a compositor")
                .contains("never falls back")
        );
    }

    #[test]
    fn canonical_unsafe_local_rejects_vm_compatibility_identity() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--target",
            "browser.host.d2b",
            "--provider-kind",
            "unsafe-local",
            "--vm-name",
            "browser",
        ])
        .expect("parse args");

        assert!(resolve_identity(&args).is_err());
    }

    #[test]
    fn unsafe_local_rejects_direct_child_mode_so_scope_lifecycles_stay_separate() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--target",
            "browser.host.d2b",
            "--provider-kind",
            "unsafe-local",
            "--host-terminal",
        ])
        .expect("parse args");
        let identity = resolve_identity(&args).unwrap();

        assert!(
            validate_execution_mode(&args, &identity)
                .expect_err("unsafe-local app must not be child-coupled to proxy")
                .contains("authenticated helper")
        );
    }

    #[test]
    fn unsafe_local_has_bounded_first_client_deadline_by_default() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--target",
            "browser.host.d2b",
            "--provider-kind",
            "unsafe-local",
            "--listen",
            "target/test.sock",
            "--connect",
            "wayland-1",
        ])
        .expect("parse args");
        let identity = resolve_identity(&args).unwrap();

        assert_eq!(
            configured_first_client_timeout(&args, &identity),
            Some(Duration::from_secs(10))
        );
        let now = Instant::now();
        assert_eq!(
            bound_poll_timeout_to_deadline(60_000, now + Duration::from_millis(25), now),
            25
        );
    }

    #[test]
    fn legacy_long_lived_vm_proxy_does_not_gain_an_initial_client_deadline() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--vm-name",
            "work",
            "--listen",
            "target/test.sock",
            "--connect",
            "wayland-1",
        ])
        .expect("parse args");
        let identity = resolve_identity(&args).unwrap();

        assert_eq!(configured_first_client_timeout(&args, &identity), None);
    }

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
    fn accept_backoff_masks_listener_and_bounds_poll_timeout() {
        let now = Instant::now();
        let backoff_until = now + Duration::from_millis(50);
        assert!(accept_backoff_active(Some(backoff_until), now));
        assert!(listener_accept_poll_flags(true).is_empty());
        assert_eq!(
            listener_accept_poll_flags(false),
            PollFlags::IN | PollFlags::ERR | PollFlags::HUP
        );
        assert_eq!(
            accept_poll_timeout_ms(
                Duration::from_secs(60),
                Some(backoff_until),
                None,
                now,
                false,
            ),
            50
        );
        assert_eq!(
            accept_poll_timeout_ms(
                Duration::from_secs(60),
                Some(now + Duration::from_secs(5)),
                Some(now + Duration::from_millis(25)),
                now,
                false,
            ),
            25
        );
    }

    #[test]
    fn expired_accept_backoff_restores_listener_interest() {
        let now = Instant::now();
        assert!(!accept_backoff_active(
            Some(now - Duration::from_millis(1)),
            now
        ));
        assert_eq!(
            accept_poll_timeout_ms(Duration::from_millis(25), Some(now), None, now, false),
            0
        );
    }

    #[test]
    fn child_watch_bounds_accept_poll_timeout() {
        let now = Instant::now();
        assert_eq!(
            accept_poll_timeout_ms(Duration::from_secs(60), None, None, now, true),
            100
        );
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

    #[test]
    fn clients_do_not_own_state_destructors() {
        let main_src = include_str!("main.rs");
        assert!(
            !main_src.contains(concat!("state.", "create_destructor("))
                && !main_src.contains(concat!("FilterClientHandler::", "with_destructor")),
            "per-client destructors terminate the long-lived proxy when a client disconnects"
        );
        let filter_src = include_str!("filter.rs");
        assert!(
            !filter_src.contains("Destructor"),
            "FilterClientHandler must not own state destructors"
        );
    }
}
