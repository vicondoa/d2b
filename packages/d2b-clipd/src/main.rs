//! d2b-clipd: host-session clipboard authority daemon.
//!
//! Connects to the host Wayland compositor via the data-control protocol,
//! subscribes to Niri IPC events for focused-window attribution, supervises
//! the picker process, and drives the native-paste fallback state machine.
//!
//! No raw clipboard contents are ever logged.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use d2b_clipd::audit::bounded_mime;
use d2b_clipd::fallback::{FallbackArming, FallbackState, FallbackTransition};
use d2b_clipd::framing::{
    OpenRequestFrameCaps, PICKER_TO_DAEMON_MAX_FRAME_BYTES, decode_frame, encode_frame,
};
use d2b_clipd::host::HostClipboard;
use d2b_clipd::niri::{
    FocusedWindowSnapshot, HostClipboardAttributor, NiriEvent, NiriIpcError, NiriJsonClient,
    NiriRequest,
};
use d2b_clipd::notifications::{
    DesktopNotifier, Notifier, emit_fallback_ready, sanitize_notification_text,
};
use d2b_clipd::picker::{
    CommandPickerSpawner, PickerCommand, PickerPoll, PickerState, PickerSupervisor,
};
use d2b_clipd::policy::ReasonCode;
use d2b_clipd::protocol::{
    AttributionQuality, Candidate, ClientHello, DaemonToPickerMessage, DestinationMetadata,
    OpenRequest, PickerToDaemonMessage, RealmKind,
};
use d2b_clipd::wayland::{DataControlClient, HostClipboardEvent};
use rustix::event::{PollFd, PollFlags, poll};
use serde::Deserialize;

const CONTROL_MAX_FRAME_BYTES: usize = 1024;
const BOUNDED_READ_TIMEOUT: Duration = Duration::from_secs(2);
const CURRENT_HOST_ENTRY_ID: &str = "current-host-selection";
const MATERIALIZE_MAX_BYTES: usize = 8 * 1024 * 1024;

// ─── CLI args ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    config: PathBuf,
    picker: Option<PathBuf>,
    bridge_root: PathBuf,
    niri_socket: Option<PathBuf>,
    check_config: bool,
    oneshot: bool,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(error) = run(std::env::args().skip(1)) {
        eprintln!("d2b-clipd: {error}");
        std::process::exit(2);
    }
}

fn run(args_iter: impl IntoIterator<Item = String>) -> Result<(), String> {
    let args = parse_args(args_iter)?;

    let config_text = std::fs::read_to_string(&args.config)
        .map_err(|e| format!("failed to read config {}: {e}", args.config.display()))?;
    let config_json: serde_json::Value = serde_json::from_str(&config_text)
        .map_err(|e| format!("invalid config JSON {}: {e}", args.config.display()))?;

    // Picker: CLI arg takes precedence, then config file key.
    let picker_from_config = config_json
        .pointer("/picker/executable")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    let picker = args.picker.clone().or(picker_from_config);
    if let Some(p) = &args.picker
        && !p.is_absolute()
    {
        return Err(format!("--picker path must be absolute: {}", p.display()));
    }
    if !args.bridge_root.is_absolute() {
        return Err(format!(
            "--bridge-root path must be absolute: {}",
            args.bridge_root.display()
        ));
    }
    if args.check_config {
        println!("d2b-clipd: config ok");
        return Ok(());
    }

    // ── Wayland data-control ─────────────────────────────────────────────────
    let mut data_control = DataControlClient::connect().map_err(|e| e.to_string())?;

    // ── Niri IPC event stream thread ─────────────────────────────────────────
    let niri_socket: Option<PathBuf> = args
        .niri_socket
        .clone()
        .or_else(|| std::env::var("NIRI_SOCKET").ok().map(PathBuf::from));
    let (niri_tx, niri_rx) = mpsc::channel::<NiriMessage>();
    if let Some(ref socket) = niri_socket {
        spawn_niri_event_thread(socket.clone(), niri_tx);
    } else {
        log::warn!("d2b-clipd: NIRI_SOCKET not set; focused-window attribution unavailable");
    }

    // ── Host clipboard state ─────────────────────────────────────────────────
    let niri_query = NiriQueryProvider::new(niri_socket);
    let attributor = HostClipboardAttributor::new(niri_query);
    let mut host_clipboard: HostClipboard<NiriQueryProvider> =
        HostClipboard::new(attributor, Duration::from_secs(30));

    // ── Picker supervisor ────────────────────────────────────────────────────
    let picker_command = picker.map(|p| PickerCommand {
        program: p.into_os_string(),
        args: vec![],
    });
    let mut supervisor = PickerSupervisor::new(CommandPickerSpawner);

    // ── Fallback state machine ───────────────────────────────────────────────
    let mut fallback = FallbackArming::default();
    let mut notifier = DesktopNotifier;

    // ── Control socket ───────────────────────────────────────────────────────
    let control_socket = control_socket_path()?;
    install_control_socket_parent(&control_socket)?;
    let listener =
        UnixListener::bind(&control_socket).map_err(|e| format!("bind control socket: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    log::info!(
        "d2b-clipd: ready (config={}, bridge_root={}, control={})",
        args.config.display(),
        args.bridge_root.display(),
        control_socket.display()
    );

    if args.oneshot {
        return Ok(());
    }

    let mut event_loop = EventLoop {
        listener: &listener,
        data_control: &mut data_control,
        niri_rx,
        host_clipboard: &mut host_clipboard,
        supervisor: &mut supervisor,
        picker_command,
        fallback: &mut fallback,
        notifier: &mut notifier,
    };
    event_loop.run()
}

// ─── Event loop ───────────────────────────────────────────────────────────────

struct EventLoop<'a> {
    listener: &'a UnixListener,
    data_control: &'a mut DataControlClient,
    niri_rx: mpsc::Receiver<NiriMessage>,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: Option<PickerCommand>,
    fallback: &'a mut FallbackArming,
    notifier: &'a mut DesktopNotifier,
}

impl EventLoop<'_> {
    fn run(&mut self) -> Result<(), String> {
        loop {
            // Flush pending Wayland requests before polling.
            self.data_control.flush().ok();

            let (wayland_ready, control_ready) = {
                let mut poll_fds = [
                    PollFd::from_borrowed_fd(
                        self.data_control.as_fd(),
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ),
                    PollFd::new(
                        self.listener,
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ),
                ];
                match poll(&mut poll_fds, 50) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => return Err(format!("poll failed: {error}")),
                }
                (
                    poll_fds[0]
                        .revents()
                        .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP),
                    poll_fds[1].revents().contains(PollFlags::IN),
                )
            };

            // ── Wayland events ────────────────────────────────────────────────
            if wayland_ready {
                self.data_control.prepare_and_read().ok();
            }
            let wl_events = self.data_control.dispatch_pending().unwrap_or_else(|e| {
                log::error!("d2b-clipd: wayland dispatch: {e}");
                vec![]
            });
            for event in wl_events {
                handle_wayland_event(
                    event,
                    self.data_control,
                    self.host_clipboard,
                    self.notifier,
                    self.fallback,
                    self.supervisor,
                    &self.picker_command,
                );
            }

            // ── Control socket accepts ────────────────────────────────────────
            if control_ready {
                loop {
                    match self.listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(error) = stream.set_nonblocking(true) {
                                log::warn!(
                                    "d2b-clipd: failed to set control stream nonblocking: {error}"
                                );
                                continue;
                            }
                            let _ = handle_control_connection(
                                stream,
                                self.supervisor,
                                &self.picker_command,
                                self.host_clipboard,
                                self.fallback,
                                self.notifier,
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) => return Err(format!("control accept failed: {error}")),
                    }
                }
            }

            // ── Picker responses ──────────────────────────────────────────────
            match self
                .supervisor
                .poll_active(PICKER_TO_DAEMON_MAX_FRAME_BYTES)
            {
                Ok(PickerPoll::Message(message)) => handle_picker_message(
                    message,
                    self.data_control,
                    self.host_clipboard,
                    self.notifier,
                    self.fallback,
                    self.supervisor,
                ),
                Ok(PickerPoll::Closed) => {
                    let _ = self.fallback.cancel_picker();
                }
                Ok(PickerPoll::Incomplete) => {}
                Err(error) => {
                    log::warn!("d2b-clipd: picker frame failed: {error}");
                    let _ = self.fallback.cancel_picker();
                    let _ = self.supervisor.cancel_active(ReasonCode::PickerCrashed);
                }
            }

            // ── Niri event channel (mpsc, drained each iteration) ─────────────
            drain_niri_channel(&self.niri_rx, self.host_clipboard, self.fallback);

            // ── Periodic timeout checks ───────────────────────────────────────
            let now = Instant::now();
            if let Some(expired) = self.host_clipboard.check_paste_timeout(now) {
                log::debug!(
                    "d2b-clipd: paste fd timed out (mime={})",
                    bounded_mime(&expired.mime_type)
                );
                expired.close_with_reason(ReasonCode::FdWriteTimeout);
            }
            if let Some(_reason) = self.supervisor.reap_expired(now) {
                let _ = self.fallback.cancel_picker();
            }
            if let FallbackTransition::Cleared(r) = self.fallback.on_timeout(now) {
                log::debug!("d2b-clipd: fallback armed state cleared: {r:?}");
            }
        }
    }
}

// ─── Wayland event handler ────────────────────────────────────────────────────

fn handle_wayland_event(
    event: HostClipboardEvent,
    data_control: &mut DataControlClient,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut DesktopNotifier,
    fallback: &mut FallbackArming,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
) {
    match event {
        HostClipboardEvent::SelectionChanged {
            offer,
            allowed_mimes,
            has_secret,
        } => {
            // A new native selection supersedes any armed fallback.
            let _ = fallback.on_native_selection_changed();
            host_clipboard.on_host_selection_changed(offer, allowed_mimes, has_secret);
        }
        HostClipboardEvent::SelectionCleared => {
            host_clipboard.on_host_selection_cleared();
        }
        HostClipboardEvent::SourceSendRequest {
            source_id: _,
            mime_type,
            fd,
        } => {
            // Host application requesting paste data.  Hold the write FD.
            match host_clipboard.accept_paste_fd(fd, mime_type.clone()) {
                Ok(dest) => {
                    log::debug!(
                        "d2b-clipd: paste fd held for mime={} dest={}",
                        bounded_mime(&mime_type),
                        bounded_label(dest.app_id.as_deref().unwrap_or("unknown"))
                    );
                    if !fulfill_armed_fallback(fallback, host_clipboard, data_control, notifier) {
                        open_picker_or_arm_fallback(
                            fallback,
                            dest,
                            host_clipboard,
                            notifier,
                            supervisor,
                            picker_command,
                        );
                    }
                }
                Err(reason) => {
                    // fd already dropped; requester will see EOF.
                    log::debug!("d2b-clipd: paste fd rejected: {}", reason.as_str());
                }
            }
        }
        HostClipboardEvent::SourceCancelled { source_id } => {
            log::debug!("d2b-clipd: source {source_id} cancelled");
        }
        HostClipboardEvent::DeviceFinished => {
            log::warn!("d2b-clipd: data-control device finished (compositor seat removed)");
        }
    }
}

// ─── Control socket handler ───────────────────────────────────────────────────

fn handle_control_connection(
    mut stream: UnixStream,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
    notifier: &mut DesktopNotifier,
) -> Result<(), String> {
    let response = match read_control_command(&stream) {
        Ok(ControlCommand::Arm) => handle_arm(
            supervisor,
            picker_command,
            host_clipboard,
            fallback,
            notifier,
        ),
        Err(error) => Err(error),
    };
    let body = match response {
        Ok(msg) => format!("{{\"ok\":true,\"message\":{}}}\n", json_string(&msg)),
        Err(err) => format!("{{\"ok\":false,\"error\":{}}}\n", json_string(&err)),
    };
    stream
        .write_all(body.as_bytes())
        .map_err(|e| format!("write control response: {e}"))
}

fn handle_picker_message(
    message: PickerToDaemonMessage,
    data_control: &mut DataControlClient,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut DesktopNotifier,
    fallback: &mut FallbackArming,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
) {
    match message {
        PickerToDaemonMessage::Select(select) => {
            log::debug!(
                "d2b-clipd: picker selected entry for request {}",
                select.request_id
            );
            if host_clipboard.pending_paste().is_some() {
                match materialize_selected_entry(host_clipboard, data_control, &select.entry_id)
                    .and_then(|bytes| host_clipboard.write_paste_data(&bytes, notifier))
                {
                    Ok(()) => {
                        let _ = fallback.cancel_picker();
                        let _ = supervisor.cancel_active(ReasonCode::Allowed);
                    }
                    Err(reason) => {
                        if let Some(paste) = host_clipboard.take_pending_paste() {
                            paste.close_with_reason(reason);
                        }
                        d2b_clipd::notifications::emit_user_visible_failure(
                            notifier,
                            reason,
                            "clipboard",
                            "host",
                        );
                        let _ = fallback.cancel_picker();
                        let _ = supervisor.cancel_active(reason);
                    }
                }
                return;
            }

            let transition = fallback.arm_selected_entry(
                select.entry_id,
                Instant::now(),
                Duration::from_secs(30),
            );
            if matches!(transition, FallbackTransition::Armed) {
                let _ = supervisor.cancel_active(ReasonCode::Allowed);
            } else {
                let _ = supervisor.cancel_active(ReasonCode::PolicyDenied);
            }
        }
        PickerToDaemonMessage::Cancel(cancel) => {
            log::debug!("d2b-clipd: picker cancelled request {}", cancel.request_id);
            if let Some(paste) = host_clipboard.take_pending_paste() {
                paste.close_with_reason(ReasonCode::PickerTimeout);
            }
            let _ = fallback.cancel_picker();
            let _ = supervisor.cancel_active(ReasonCode::PickerTimeout);
        }
        PickerToDaemonMessage::ClientHello(_) => {
            log::debug!("d2b-clipd: ignored duplicate picker client_hello");
        }
    }
}

fn fulfill_armed_fallback(
    fallback: &mut FallbackArming,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    data_control: &mut DataControlClient,
    notifier: &mut DesktopNotifier,
) -> bool {
    let FallbackState::Armed {
        entry_id,
        expires_at,
        ..
    } = fallback.state().clone()
    else {
        return false;
    };
    if Instant::now() >= expires_at {
        let _ = fallback.on_timeout(Instant::now());
        return false;
    }
    let result = materialize_selected_entry(host_clipboard, data_control, &entry_id)
        .and_then(|bytes| host_clipboard.write_paste_data(&bytes, notifier));
    match result {
        Ok(()) => {
            let _ = fallback.cancel_picker();
            true
        }
        Err(reason) => {
            if let Some(paste) = host_clipboard.take_pending_paste() {
                paste.close_with_reason(reason);
            }
            d2b_clipd::notifications::emit_user_visible_failure(
                notifier,
                reason,
                "clipboard",
                "host",
            );
            let _ = fallback.cancel_picker();
            true
        }
    }
}

fn materialize_selected_entry(
    host_clipboard: &HostClipboard<NiriQueryProvider>,
    data_control: &mut DataControlClient,
    entry_id: &str,
) -> Result<Vec<u8>, ReasonCode> {
    if entry_id != CURRENT_HOST_ENTRY_ID {
        return Err(ReasonCode::PolicyDenied);
    }
    let selection = host_clipboard
        .current_selection()
        .ok_or(ReasonCode::RequestExpired)?;
    let offer = selection.offer.as_ref().ok_or(ReasonCode::PolicyDenied)?;
    let requested_mime = host_clipboard
        .pending_paste()
        .map(|paste| paste.mime_type.as_str())
        .ok_or(ReasonCode::IntentMissing)?;
    if !selection
        .allowed_mimes
        .iter()
        .any(|mime| mime == requested_mime)
    {
        return Err(ReasonCode::MimeRejected);
    }
    let (read_fd, write_fd) = rustix::pipe::pipe().map_err(|_| ReasonCode::FdClosed)?;
    offer.receive(requested_mime.to_owned(), &write_fd);
    data_control
        .flush()
        .map_err(|_| ReasonCode::BridgeUnavailable)?;
    drop(write_fd);
    read_fd_to_vec(read_fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT)
}

fn read_fd_to_vec(
    fd: std::os::fd::OwnedFd,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, ReasonCode> {
    use std::os::fd::AsFd;

    rustix::io::ioctl_fionbio(fd.as_fd(), true).map_err(|_| ReasonCode::FdClosed)?;
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    loop {
        let mut buf = [0_u8; 4096];
        match rustix::io::read(&fd, &mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                if out.len().saturating_add(n) > max_bytes {
                    return Err(ReasonCode::MemoryCapExceeded);
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                wait_readable(&fd, deadline).map_err(|_| ReasonCode::SourceMaterializeTimeout)?;
            }
            Err(_) => return Err(ReasonCode::FdClosed),
        }
    }
}

/// `d2b clipboard picker` sends `{"type":"arm"}` to this socket.
/// We open the picker (if configured) and arm the native-paste fallback for
/// the current focused window.
fn handle_arm(
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
    notifier: &mut DesktopNotifier,
) -> Result<String, String> {
    let dest = host_clipboard
        .current_selection()
        .and_then(|s| s.attribution.window.clone())
        .unwrap_or_default();

    let request_id = format!("arm-{}", unix_millis());
    let ambient: BTreeMap<OsString, OsString> = std::env::vars_os().collect();

    // Capture the intended target before opening the picker so focus
    // restoration back to the target after picker close is ignored.
    let _ = fallback.capture_target_before_picker(dest.clone());

    match supervisor.launch(
        request_id.clone(),
        picker_command.clone(),
        &ambient,
        Duration::from_secs(30),
    ) {
        Ok(socket) => {
            // Drive picker handshake: read ClientHello, send OpenRequest.
            let candidates = picker_candidates(host_clipboard);
            match picker_handshake(socket, &request_id, &dest, candidates) {
                Ok(picker_version) => {
                    log::debug!("d2b-clipd: picker opened (version={picker_version})");
                    Ok("picker opened".to_owned())
                }
                Err(error) => {
                    log::warn!("d2b-clipd: picker handshake failed: {error}");
                    let _ = supervisor.cancel_active(ReasonCode::PickerCrashed);
                    let _ = fallback.cancel_picker();
                    Err(error)
                }
            }
        }
        Err(e) => {
            log::warn!("d2b-clipd: picker launch failed: {e}; arming native fallback");
            let _ = fallback.cancel_picker();
            arm_native_fallback(fallback, dest.clone(), notifier);
            Err(e.to_string())
        }
    }
}

/// Perform the picker ClientHello / OpenRequest handshake.
/// Returns the picker version string on success.
fn picker_handshake(
    socket: &UnixStream,
    request_id: &str,
    dest: &FocusedWindowSnapshot,
    candidates: Vec<Candidate>,
) -> Result<String, String> {
    let hello_buf = read_bounded_line(
        socket,
        PICKER_TO_DAEMON_MAX_FRAME_BYTES,
        BOUNDED_READ_TIMEOUT,
    )
    .map_err(|e| format!("read hello: {e}"))?;
    let hello: PickerToDaemonMessage = decode_frame(&hello_buf, PICKER_TO_DAEMON_MAX_FRAME_BYTES)
        .map_err(|e| format!("decode hello: {e}"))?;
    let picker_version = match hello {
        PickerToDaemonMessage::ClientHello(ClientHello { picker_version, .. }) => picker_version,
        _ => return Err("first frame was not client_hello".to_owned()),
    };

    let request = DaemonToPickerMessage::OpenRequest(Box::new(OpenRequest {
        selected_protocol_version: 1,
        clipd_version: env!("CARGO_PKG_VERSION").to_owned(),
        picker_version: picker_version.clone(),
        request_id: request_id.to_owned(),
        destination: DestinationMetadata {
            realm: "Host".to_owned(),
            realm_kind: RealmKind::Host,
            application: dest.app_id.clone(),
            app_id: dest.app_id.clone(),
            title: dest.title.clone(),
            workspace: None,
            output: dest.output_label.clone(),
            attribution: AttributionQuality::FocusedWindowGuess,
        },
        requested_mime_type: "text/plain".to_owned(),
        expires_at_unix_ms: unix_millis().saturating_add(30_000),
        placement_hints: None,
        candidates,
    }));
    let frame = encode_frame(&request, OpenRequestFrameCaps::default().max_frame_bytes())
        .map_err(|e| format!("encode open_request: {e}"))?;
    socket
        .try_clone()
        .map_err(|e| format!("clone for write: {e}"))?
        .write_all(&frame)
        .map_err(|e| format!("write open_request: {e}"))?;

    Ok(picker_version)
}

fn picker_candidates(host_clipboard: &HostClipboard<NiriQueryProvider>) -> Vec<Candidate> {
    let Some(selection) = host_clipboard.current_selection() else {
        return Vec::new();
    };
    if selection.offer.is_none() || selection.allowed_mimes.is_empty() {
        return Vec::new();
    }
    let window = selection.attribution.window.as_ref();
    vec![Candidate {
        entry_id: CURRENT_HOST_ENTRY_ID.to_owned(),
        source_realm: "Host".to_owned(),
        source_realm_kind: RealmKind::Host,
        source_app: window
            .and_then(|window| window.title.clone())
            .or_else(|| Some("Host clipboard".to_owned())),
        source_app_id: window.and_then(|window| window.app_id.clone()),
        source_attribution: protocol_attribution(selection.attribution.quality),
        preview_text: None,
        content_type: selection
            .allowed_mimes
            .first()
            .cloned()
            .unwrap_or_else(|| "text/plain".to_owned()),
        timestamp_unix_ms: unix_millis(),
        thumbnail_png_base64: None,
        byte_count: None,
        confirmation_required: false,
    }]
}

fn protocol_attribution(quality: d2b_clipd::policy::AttributionQuality) -> AttributionQuality {
    match quality {
        d2b_clipd::policy::AttributionQuality::ExactClient => AttributionQuality::ExactClient,
        d2b_clipd::policy::AttributionQuality::FocusedWindowGuess => {
            AttributionQuality::FocusedWindowGuess
        }
        d2b_clipd::policy::AttributionQuality::CacheStaleFocusedWindowGuess => {
            AttributionQuality::CacheStaleFocusedWindowGuess
        }
        d2b_clipd::policy::AttributionQuality::BrokerInjectedDebug => {
            AttributionQuality::BrokerInjectedDebug
        }
    }
}

// ─── Picker / fallback helpers ────────────────────────────────────────────────

fn open_picker_or_arm_fallback(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    host_clipboard: &HostClipboard<NiriQueryProvider>,
    notifier: &mut impl Notifier,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
) {
    let can_open = picker_command.is_some() && matches!(supervisor.state(), PickerState::Idle);
    if can_open {
        let _ = fallback.capture_target_before_picker(dest.clone());
        let ambient: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        let request_id = format!("paste-{}", unix_millis());
        match supervisor.launch(
            request_id.clone(),
            picker_command.clone(),
            &ambient,
            Duration::from_secs(30),
        ) {
            Ok(socket) => {
                let candidates = picker_candidates(host_clipboard);
                if let Err(error) = picker_handshake(socket, &request_id, &dest, candidates) {
                    log::warn!("d2b-clipd: picker handshake failed: {error}");
                    let _ = supervisor.cancel_active(ReasonCode::PickerCrashed);
                    let _ = fallback.cancel_picker();
                    arm_native_fallback(fallback, dest, notifier);
                } else {
                    log::debug!(
                        "d2b-clipd: picker opened for paste to {}",
                        bounded_label(dest.app_id.as_deref().unwrap_or("unknown"))
                    );
                }
            }
            Err(e) => {
                log::warn!("d2b-clipd: picker launch failed ({e}); falling back to native paste");
                let _ = fallback.cancel_picker();
                arm_native_fallback(fallback, dest, notifier);
            }
        }
    } else {
        arm_native_fallback(fallback, dest, notifier);
    }
}

fn arm_native_fallback(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    notifier: &mut impl Notifier,
) {
    if matches!(fallback.state(), FallbackState::Idle) {
        let _ = fallback.capture_target_before_picker(dest.clone());
    }
    let transition = fallback.arm_selected_entry(
        CURRENT_HOST_ENTRY_ID.to_owned(),
        Instant::now(),
        Duration::from_secs(30),
    );
    if matches!(transition, FallbackTransition::Armed) {
        let label = dest
            .app_id
            .as_deref()
            .or(dest.title.as_deref())
            .unwrap_or("host application");
        emit_fallback_ready(notifier, label);
        log::debug!(
            "d2b-clipd: fallback armed for {}; user should press Ctrl+V",
            bounded_label(label)
        );
    }
}

// ─── Niri event thread ────────────────────────────────────────────────────────

#[derive(Debug)]
enum NiriMessage {
    Event(NiriEvent),
    Disconnected,
}

fn spawn_niri_event_thread(socket: PathBuf, tx: mpsc::Sender<NiriMessage>) {
    std::thread::Builder::new()
        .name("d2b-clipd-niri".to_owned())
        .spawn(move || {
            let mut client = match NiriJsonClient::connect(
                &socket,
                d2b_clipd::niri::DEFAULT_NIRI_MAX_LINE_BYTES,
                Some(Duration::from_secs(5)),
            ) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("d2b-clipd: niri connect: {e}");
                    let _ = tx.send(NiriMessage::Disconnected);
                    return;
                }
            };

            // Send EventStream request and read the initial OK acknowledgement.
            let _: serde_json::Value = match client.request(&NiriRequest::EventStream) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("d2b-clipd: niri EventStream: {e}");
                    let _ = tx.send(NiriMessage::Disconnected);
                    return;
                }
            };

            // Stream events continuously.
            loop {
                match client.read_event() {
                    Ok(event) => {
                        if tx.send(NiriMessage::Event(event)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("d2b-clipd: niri event: {e}");
                        let _ = tx.send(NiriMessage::Disconnected);
                        break;
                    }
                }
            }
        })
        .expect("niri thread spawn");
}

fn drain_niri_channel(
    rx: &mpsc::Receiver<NiriMessage>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
) {
    for _ in 0..64 {
        match rx.try_recv() {
            Ok(NiriMessage::Event(event)) => {
                host_clipboard.apply_niri_cache_event(event.clone());
                if let NiriEvent::FocusChanged { .. } = &event {
                    // The updated cache is reflected the next time attribution is queried.
                    let snapshot = host_clipboard
                        .current_selection()
                        .and_then(|s| s.attribution.window.clone());
                    let _ = fallback.on_focus_changed(snapshot);
                }
            }
            Ok(NiriMessage::Disconnected) => {
                log::warn!("d2b-clipd: niri stream disconnected");
                break;
            }
            Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

// ─── Niri query provider (for on-demand attribution) ─────────────────────────

pub struct NiriQueryProvider {
    socket: Option<PathBuf>,
}

impl NiriQueryProvider {
    fn new(socket: Option<PathBuf>) -> Self {
        Self { socket }
    }
}

impl d2b_clipd::niri::FocusedWindowProvider for NiriQueryProvider {
    fn query_focused_window(
        &mut self,
    ) -> Result<Option<d2b_clipd::niri::NiriWindow>, NiriIpcError> {
        let Some(ref socket) = self.socket else {
            return Ok(None);
        };
        let mut client = NiriJsonClient::connect(
            socket,
            d2b_clipd::niri::DEFAULT_NIRI_MAX_LINE_BYTES,
            Some(Duration::from_secs(2)),
        )?;
        client.query_focused_window()
    }
}

// ─── Control socket helpers ───────────────────────────────────────────────────

fn control_socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR is required for d2b-clipd control socket".to_owned())?;
    Ok(PathBuf::from(runtime).join("d2b-clipd/clipd.sock"))
}

fn install_control_socket_parent(socket: &Path) -> Result<(), String> {
    let parent = socket
        .parent()
        .ok_or_else(|| format!("control socket has no parent: {}", socket.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create control socket dir {}: {e}", parent.display()))?;
    let _ = std::fs::remove_file(socket);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlCommand {
    Arm,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ControlFrame {
    Arm,
}

fn read_control_command(stream: &UnixStream) -> Result<ControlCommand, String> {
    let line = read_bounded_line(stream, CONTROL_MAX_FRAME_BYTES, BOUNDED_READ_TIMEOUT)?;
    match serde_json::from_slice::<ControlFrame>(&line)
        .map_err(|e| format!("invalid control JSON: {e}"))?
    {
        ControlFrame::Arm => Ok(ControlCommand::Arm),
    }
}

fn read_bounded_line(
    stream: &UnixStream,
    max_frame_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    let mut stream = stream
        .try_clone()
        .map_err(|e| format!("clone stream: {e}"))?;
    let mut out = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => return Err("peer closed before newline".to_owned()),
            Ok(_) => {
                out.push(byte[0]);
                if out.len() > max_frame_bytes {
                    return Err(format!("frame exceeds {max_frame_bytes} bytes"));
                }
                if byte[0] == b'\n' {
                    return Ok(out);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait_readable(&stream, deadline)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn wait_readable<Fd: std::os::fd::AsFd>(fd: &Fd, deadline: Instant) -> Result<(), String> {
    let now = Instant::now();
    if now >= deadline {
        return Err("timed out waiting for readability".to_owned());
    }
    let timeout = deadline
        .saturating_duration_since(now)
        .as_millis()
        .min(i32::MAX as u128) as i32;
    let mut fds = [PollFd::new(
        fd,
        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
    )];
    match poll(&mut fds, timeout) {
        Ok(0) => Err("timed out waiting for readability".to_owned()),
        Ok(_) if fds[0].revents().intersects(PollFlags::ERR | PollFlags::HUP) => {
            Err("fd closed while waiting for readability".to_owned())
        }
        Ok(_) => Ok(()),
        Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

// ─── Utility ─────────────────────────────────────────────────────────────────

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"serialization-error\"".to_owned())
}

fn bounded_label(label: &str) -> String {
    sanitize_notification_text(label, 80)
}

// ─── Arg parsing ─────────────────────────────────────────────────────────────

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut config = None;
    let mut picker = None;
    let mut bridge_root = None;
    let mut niri_socket = None;
    let mut check_config = false;
    let mut oneshot = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                config = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--config requires a path".to_owned())?,
                ));
            }
            "--picker" => {
                picker = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--picker requires a path".to_owned())?,
                ));
            }
            "--bridge-root" => {
                bridge_root = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--bridge-root requires a path".to_owned())?,
                ));
            }
            "--niri-socket" => {
                niri_socket = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--niri-socket requires a path".to_owned())?,
                ));
            }
            "--check-config" => check_config = true,
            "--oneshot" => oneshot = true,
            "--help" | "-h" => {
                return Err("usage: d2b-clipd --config <path> --bridge-root <path> \
                     [--picker <path>] [--niri-socket <path>] [--check-config] [--oneshot]"
                    .to_owned());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        config: config.ok_or_else(|| "--config is required".to_owned())?,
        picker,
        bridge_root: bridge_root.ok_or_else(|| "--bridge-root is required".to_owned())?,
        niri_socket,
        check_config,
        oneshot,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_required_args() {
        let args = parse_args([
            "--config".to_owned(),
            "/etc/d2b/clipboard.json".to_owned(),
            "--bridge-root".to_owned(),
            "/run/d2b/clipd".to_owned(),
            "--check-config".to_owned(),
            "--oneshot".to_owned(),
        ])
        .expect("args");
        assert_eq!(args.config, PathBuf::from("/etc/d2b/clipboard.json"));
        assert_eq!(args.bridge_root, PathBuf::from("/run/d2b/clipd"));
        assert!(args.check_config);
        assert!(args.oneshot);
        assert!(args.niri_socket.is_none());
    }

    #[test]
    fn parses_niri_socket_arg() {
        let args = parse_args([
            "--config".to_owned(),
            "/etc/d2b/clipboard.json".to_owned(),
            "--bridge-root".to_owned(),
            "/run/d2b/clipd".to_owned(),
            "--niri-socket".to_owned(),
            "/run/user/1000/niri.sock".to_owned(),
            "--oneshot".to_owned(),
        ])
        .expect("args");
        assert_eq!(
            args.niri_socket,
            Some(PathBuf::from("/run/user/1000/niri.sock"))
        );
    }

    #[test]
    fn rejects_unknown_args() {
        let err = parse_args(["--wat".to_owned()]).expect_err("unknown");
        assert!(err.contains("unknown argument"));
    }

    #[test]
    fn control_command_rejects_malformed_json() {
        let (mut writer, reader) = UnixStream::pair().expect("pair");
        writer.write_all(b"{not-json}\n").expect("write");
        let err = read_control_command(&reader).expect_err("malformed");
        assert!(err.contains("invalid control JSON"));
    }

    #[test]
    fn bounded_line_rejects_overlong_control_frame() {
        let (mut writer, reader) = UnixStream::pair().expect("pair");
        let bytes = vec![b'a'; CONTROL_MAX_FRAME_BYTES + 1];
        writer.write_all(&bytes).expect("write");
        let err = read_bounded_line(&reader, CONTROL_MAX_FRAME_BYTES, Duration::from_secs(1))
            .expect_err("overlong");
        assert!(err.contains("frame exceeds"));
    }

    #[test]
    fn bounded_line_times_out_on_hanging_frame() {
        let (_writer, reader) = UnixStream::pair().expect("pair");
        reader.set_nonblocking(true).expect("nonblocking");
        let err = read_bounded_line(&reader, CONTROL_MAX_FRAME_BYTES, Duration::from_millis(5))
            .expect_err("timeout");
        assert!(err.contains("timed out"));
    }

    #[test]
    fn fd_materialization_times_out_on_hanging_stream() {
        let (read_fd, _write_fd) = rustix::pipe::pipe().expect("pipe");
        let err = read_fd_to_vec(read_fd, 1024, Duration::from_millis(5)).expect_err("timeout");
        assert_eq!(err, ReasonCode::SourceMaterializeTimeout);
    }
}
