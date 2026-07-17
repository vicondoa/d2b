//! ComponentSession-owned Wayland proxy process.

use std::{
    cell::RefCell,
    io,
    os::{fd::OwnedFd, unix::net::UnixListener},
    rc::Rc,
    time::{Duration, Instant},
};

use clap::Parser;
use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use d2b_wayland_proxy::{
    decoration::{BorderConfig, Color, DecorationManager, LabelPosition, sanitize_label},
    diag::{DiagRateLimiter, bounded_error_detail},
    dmabuf::{DmabufFilter, parse_filter as parse_dmabuf_filter},
    filter::{FilterStateHandler, VirtualClipboardState, build_state, install_client_handlers},
    identity::ProxyIdentity,
    policy::{FilterPolicy, PolicyInput},
    services::{
        open_authenticated_display,
        wayland::{
            AuthenticatedDescriptor, ControlError, ControlMethod, ControlRequest,
            DESCRIPTOR_CREDIT_CLASSES, DescriptorAccess, DescriptorKind, DescriptorObject,
            DescriptorPurpose, DisplayHealth, DisplayProviderBinding, DisplayProviderPort,
            OPEN_DISPLAY_METHOD_ID, Observation, ObservationSink, OpaqueId, SessionContract,
            SessionIdentity, TransportContract, WaylandControlService,
        },
    },
};
use env_logger::Env;
use rustix::event::{PollFd, PollFlags, poll};

const ACCEPT_RESOURCE_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Parser, Debug)]
#[command(name = "d2b-wayland-proxy")]
#[command(about = "Authenticated Wayland proxy for d2b workloads")]
struct Args {
    /// Authenticated ComponentSession reconnect generation.
    #[arg(long, value_parser = parse_positive_u64)]
    session_generation: u64,

    /// Canonical workload target.
    #[arg(long)]
    target: String,

    /// Provider kind for the target.
    #[arg(long)]
    provider_kind: String,

    #[arg(long)]
    realm_id: String,

    #[arg(long)]
    workload_id: String,

    #[arg(long)]
    provider_id: String,

    #[arg(long, default_value = "wayland-proxy")]
    role_id: String,

    #[arg(long)]
    app_id_prefix: Option<String>,

    #[arg(long)]
    title_prefix: Option<String>,

    #[arg(long = "deny-global")]
    deny_globals: Vec<String>,

    #[arg(long = "allow-global")]
    allow_globals: Vec<String>,

    #[arg(long = "max-version")]
    max_versions: Vec<String>,

    #[arg(long = "dmabuf-allow")]
    dmabuf_allow: Vec<String>,

    #[arg(long = "dmabuf-deny")]
    dmabuf_deny: Vec<String>,

    #[arg(long)]
    log_filtered_globals: bool,

    #[arg(long = "border-enable", default_value_t = false)]
    border_enable: bool,

    #[arg(long = "border-color-active", default_value = "#3caaff")]
    border_color_active: Color,

    #[arg(long = "border-color-inactive", default_value = "#5f5f5f")]
    border_color_inactive: Color,

    #[arg(long = "border-color-urgent", default_value = "#ff5656")]
    border_color_urgent: Color,

    #[arg(long = "border-thickness", value_parser = parse_positive_u32, default_value_t = d2b_wayland_proxy::decoration::DEFAULT_BORDER_THICKNESS)]
    border_thickness: u32,

    #[arg(long = "border-label")]
    border_label: Option<String>,

    #[arg(long = "border-label-position", default_value = "top-left")]
    border_label_position: LabelPosition,

    /// Bounded deadline for the first authenticated client.
    #[arg(long = "first-client-timeout-ms", value_parser = parse_positive_u64, default_value_t = 10_000)]
    first_client_timeout_ms: u64,
}

#[derive(Debug)]
struct DisplayPort {
    opened: bool,
}

impl DisplayProviderPort for DisplayPort {
    fn open(
        &mut self,
        _: &SessionIdentity,
        _: &DisplayProviderBinding,
        _: &ControlRequest,
    ) -> Result<OpaqueId, ControlError> {
        if self.opened {
            return Err(ControlError::ProviderUnavailable);
        }
        self.opened = true;
        OpaqueId::parse("display")
    }

    fn inspect(
        &self,
        _: &SessionIdentity,
        _: &DisplayProviderBinding,
        _: &OpaqueId,
    ) -> Result<DisplayHealth, ControlError> {
        Ok(if self.opened {
            DisplayHealth::Ready
        } else {
            DisplayHealth::Unavailable
        })
    }

    fn close(
        &mut self,
        _: &SessionIdentity,
        _: &DisplayProviderBinding,
        _: &ControlRequest,
    ) -> Result<(), ControlError> {
        self.opened = false;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct LogObservations;

impl ObservationSink for LogObservations {
    fn record(&mut self, observation: Observation) {
        log::debug!(
            "[d2b-wlproxy] event=control operation={:?} outcome={:?}",
            observation.operation,
            observation.outcome
        );
    }
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    if let Err(error) = run(Args::parse()) {
        eprintln!("d2b-wayland-proxy: {error}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), String> {
    let session_identity = session_identity(&args)?;
    let identity = proxy_identity(&args, session_identity.clone())?;
    let max_versions = args
        .max_versions
        .iter()
        .map(|value| parse_max_version(value))
        .collect::<Result<Vec<_>, _>>()?;
    let dmabuf_allow = parse_dmabuf_filters(&args.dmabuf_allow)?;
    let dmabuf_deny = parse_dmabuf_filters(&args.dmabuf_deny)?;
    let policy = FilterPolicy::build(PolicyInput {
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
    });
    for warning in &policy.warnings {
        eprintln!("d2b-wayland-proxy: warning: {}", warning.message());
    }

    let upstream = rustix::io::fcntl_dupfd_cloexec(std::io::stdin(), 3)
        .map_err(|error| format!("cannot adopt ComponentSession upstream descriptor: {error}"))?;
    let listener = rustix::io::fcntl_dupfd_cloexec(std::io::stdout(), 3)
        .map_err(|error| format!("cannot adopt ComponentSession listener descriptor: {error}"))?;
    let request_id = request_token(args.session_generation, 0x41);
    let operation_id = request_token(args.session_generation, 0x82);
    let request = open_request(
        args.session_generation,
        request_id,
        operation_id,
        OpaqueId::parse("display").map_err(|error| error.to_string())?,
    );
    let session = SessionContract {
        identity: session_identity,
        generation: args.session_generation,
        transport: TransportContract::COMPONENT_SESSION_LOCAL,
    };
    let binding = display_binding(args.session_generation)?;
    let mut control = WaylandControlService::new(
        session,
        binding,
        DisplayPort { opened: false },
        LogObservations,
    )
    .map_err(|error| error.to_string())?;
    let display = open_authenticated_display(&mut control, request, upstream, listener)
        .map_err(|error| error.to_string())?;

    let upstream = Rc::new(
        rustix::io::fcntl_dupfd_cloexec(display.fds.upstream(), 3)
            .map_err(|error| format!("cannot retain upstream descriptor: {error}"))?,
    );
    let listener_fd = rustix::io::fcntl_dupfd_cloexec(display.fds.listener(), 3)
        .map_err(|error| format!("cannot retain listener descriptor: {error}"))?;
    let listener = UnixListener::from(listener_fd);
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure listener descriptor: {error}"))?;

    let border_label = args
        .border_label
        .as_deref()
        .and_then(sanitize_label)
        .or_else(|| {
            (identity.provider_kind() == WorkloadProviderKind::UnsafeLocal)
                .then(|| identity.default_warning_label())
                .and_then(|label| sanitize_label(&label))
        });
    let border = BorderConfig {
        enabled: args.border_enable,
        active: args.border_color_active,
        inactive: args.border_color_inactive,
        urgent: args.border_color_urgent,
        thickness: args.border_thickness,
        label: border_label,
        label_position: args.border_label_position,
    };

    accept_loop(
        listener,
        upstream,
        policy,
        border,
        Duration::from_millis(args.first_client_timeout_ms),
    )
}

fn accept_loop(
    listener: UnixListener,
    upstream: Rc<OwnedFd>,
    policy: FilterPolicy,
    border: BorderConfig,
    first_client_timeout: Duration,
) -> Result<(), String> {
    let policy = Rc::new(policy);
    let target = policy.vm_name.clone();
    let diag = Rc::new(RefCell::new(DiagRateLimiter::new(target.clone())));
    let clipboard = Rc::new(RefCell::new(VirtualClipboardState::new(
        policy.identity.clone(),
        diag.clone(),
        None,
    )));
    let decoration = border
        .enabled()
        .then(|| Rc::new(RefCell::new(DecorationManager::new(border, diag.clone()))));
    let state = build_state(&upstream)
        .map_err(|error| format!("authenticated upstream descriptor is unavailable: {error}"))?;
    state.set_handler(FilterStateHandler::new(
        policy.clone(),
        diag.clone(),
        clipboard.clone(),
        decoration.clone(),
    ));

    let deadline = Instant::now() + first_client_timeout;
    let mut first_client = false;
    let mut next_client_id = 1_u64;
    let mut listener_backoff_until = None;
    let mut last_diag_flush = Instant::now();
    while state.is_not_destroyed() {
        let now = Instant::now();
        if !first_client && now >= deadline {
            return Err("first authenticated Wayland client readiness deadline expired".to_owned());
        }
        let in_backoff = listener_backoff_until.is_some_and(|until| until > now);
        if !in_backoff {
            listener_backoff_until = None;
        }
        let mut poll_fds = [
            PollFd::new(
                &listener,
                if in_backoff {
                    PollFlags::empty()
                } else {
                    PollFlags::IN | PollFlags::ERR | PollFlags::HUP
                },
            ),
            PollFd::new(state.poll_fd(), PollFlags::IN),
        ];
        let until_deadline = if first_client {
            Duration::from_secs(60)
        } else {
            deadline.saturating_duration_since(now)
        };
        let timeout = Duration::from_secs(60)
            .min(until_deadline)
            .as_millis()
            .min(i32::MAX as u128) as i32;
        match poll(&mut poll_fds, timeout) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(format!("dispatch poll failed: {error}")),
        }

        if !in_backoff && poll_fds[0].revents().contains(PollFlags::IN) {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true).map_err(|error| {
                            format!("accepted client descriptor is invalid: {error}")
                        })?;
                        let fd = Rc::new(stream.into());
                        let client = state.add_client(&fd).map_err(|error| {
                            bounded_error_detail(format!(
                                "cannot add authenticated client {next_client_id}: {error}"
                            ))
                        })?;
                        install_client_handlers(
                            &client,
                            policy.clone(),
                            diag.clone(),
                            clipboard.clone(),
                            decoration.clone(),
                        );
                        next_client_id = next_client_id.saturating_add(1);
                        first_client = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if is_recoverable_accept_error(&error) => {
                        if is_resource_exhaustion_accept_error(&error) {
                            listener_backoff_until = Some(Instant::now() + ACCEPT_RESOURCE_BACKOFF);
                        }
                        break;
                    }
                    Err(error) => return Err(format!("client accept failed: {error}")),
                }
            }
        }
        state
            .dispatch(Some(Duration::ZERO))
            .map_err(|error| bounded_error_detail(format!("Wayland dispatch failed: {error}")))?;
        VirtualClipboardState::drive_bridge_io(&clipboard);
        if last_diag_flush.elapsed() >= Duration::from_secs(60) {
            diag.borrow_mut().flush_suppressed();
            last_diag_flush = Instant::now();
        }
    }
    state.destroy();
    diag.borrow_mut().flush_suppressed();
    Ok(())
}

fn session_identity(args: &Args) -> Result<SessionIdentity, String> {
    Ok(SessionIdentity {
        realm_id: OpaqueId::parse(args.realm_id.clone()).map_err(|error| error.to_string())?,
        workload_id: OpaqueId::parse(args.workload_id.clone())
            .map_err(|error| error.to_string())?,
        provider_id: OpaqueId::parse(args.provider_id.clone())
            .map_err(|error| error.to_string())?,
        role_id: OpaqueId::parse(args.role_id.clone()).map_err(|error| error.to_string())?,
    })
}

fn proxy_identity(args: &Args, session: SessionIdentity) -> Result<ProxyIdentity, String> {
    let target = WorkloadTarget::parse(&args.target)
        .map_err(|_| "--target must be a canonical workload target".to_owned())?;
    let provider = parse_provider_kind(&args.provider_kind)?;
    let identity = ProxyIdentity::from_component_session(target, provider, session);
    let session = identity
        .require_component_session()
        .map_err(|error| error.to_string())?;
    if session.role_id.as_str() != "wayland-proxy"
        || session.workload_id.as_str() != identity.bridge_component()
    {
        return Err("ComponentSession identity does not own this Wayland endpoint".to_owned());
    }
    Ok(identity)
}

fn display_binding(generation: u64) -> Result<DisplayProviderBinding, String> {
    let parse = |value| OpaqueId::parse(value).map_err(|error| error.to_string());
    Ok(DisplayProviderBinding {
        display_endpoint_id: parse("display-endpoint")?,
        cross_domain_endpoint_id: parse("cross-domain-endpoint")?,
        waypipe_endpoint_id: parse("waypipe-endpoint")?,
        proxy_endpoint_id: parse("proxy-endpoint")?,
        resource_generation: generation,
        wayland: true,
        cross_domain: true,
        waypipe: true,
        proxy: true,
        authorization: true,
    })
}

fn open_request(
    generation: u64,
    request_id: [u8; 16],
    operation_id: [u8; 16],
    resource_id: OpaqueId,
) -> ControlRequest {
    let descriptor = |index, purpose| AuthenticatedDescriptor {
        index,
        kind: DescriptorKind::FileDescriptor,
        object: DescriptorObject::WaylandSocket,
        access: DescriptorAccess::ReadWrite,
        purpose,
        service_package: d2b_wayland_proxy::services::wayland::SERVICE_PACKAGE,
        method_id: OPEN_DISPLAY_METHOD_ID,
        request_id,
        operation_id,
        packet_sequence: 1,
        reconnect_generation: generation,
        cloexec_required: true,
        duplicate_object_allowed: false,
        credit_classes: DESCRIPTOR_CREDIT_CLASSES,
    };
    ControlRequest {
        method: ControlMethod::OpenDisplay,
        request_id,
        operation_id,
        session_generation: generation,
        resource_id,
        descriptors: vec![
            descriptor(0, DescriptorPurpose::Wayland),
            descriptor(1, DescriptorPurpose::Listener),
        ],
    }
}

fn request_token(generation: u64, domain: u8) -> [u8; 16] {
    let mut token = [0_u8; 16];
    token[..8].copy_from_slice(&generation.to_be_bytes());
    token[8] = domain;
    token
}

fn parse_max_version(value: &str) -> Result<(String, u32), String> {
    let (interface, version) = value
        .split_once('=')
        .ok_or_else(|| format!("expected INTERFACE=VERSION, got `{value}`"))?;
    let version = version
        .parse()
        .map_err(|_| format!("invalid version in `{value}`"))?;
    Ok((interface.to_owned(), version))
}

fn parse_dmabuf_filters(values: &[String]) -> Result<Vec<DmabufFilter>, String> {
    values
        .iter()
        .map(|value| parse_dmabuf_filter(value))
        .collect()
}

fn parse_positive_u32(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "expected a positive u32".to_owned())
}

fn parse_positive_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "expected a positive u64".to_owned())
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

fn is_recoverable_accept_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
        || matches!(
            error.raw_os_error(),
            Some(
                libc::ECONNABORTED
                    | libc::EINTR
                    | libc::EMFILE
                    | libc::ENFILE
                    | libc::ENOBUFS
                    | libc::ENOMEM
            )
        )
}

fn is_resource_exhaustion_accept_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_descriptor_contract_is_exact() {
        let request = open_request(
            4,
            request_token(4, 0x41),
            request_token(4, 0x82),
            OpaqueId::parse("display").unwrap(),
        );
        assert_eq!(request.descriptors.len(), 2);
        assert_eq!(request.descriptors[0].purpose, DescriptorPurpose::Wayland);
        assert_eq!(request.descriptors[1].purpose, DescriptorPurpose::Listener);
        assert!(request.descriptors.iter().all(|item| item.cloexec_required));
    }

    #[test]
    fn readiness_deadline_is_always_positive() {
        let args = Args::try_parse_from([
            "d2b-wayland-proxy",
            "--session-generation",
            "2",
            "--target",
            "work.host.d2b",
            "--provider-kind",
            "local-vm",
            "--realm-id",
            "local",
            "--workload-id",
            "endpoint-1",
            "--provider-id",
            "local-vm",
        ])
        .unwrap();
        assert_eq!(args.first_client_timeout_ms, 10_000);
    }
}
