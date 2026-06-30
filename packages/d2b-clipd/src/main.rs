//! d2b-clipd: host-session clipboard authority daemon.
//!
//! Connects to the host Wayland compositor via the data-control protocol,
//! subscribes to Niri IPC events for focused-window attribution, supervises
//! the picker process, and drives the native-paste fallback state machine.
//!
//! No raw clipboard contents are ever logged.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use d2b_clipd::audit::{
    AuditDecision, AuditEvent, AuditQueue, AuditQueueConfig, MetricEvent, MetricName, MetricsQueue,
    bounded_mime,
};
use d2b_clipd::fallback::{FallbackArming, FallbackState, FallbackTransition};
use d2b_clipd::fd::{FdCapModel, classify_fd, validate_fd_cap};
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
use d2b_clipd::policy::{ReasonCode, is_mime_allowed};
use d2b_clipd::protocol::{
    AttributionQuality, Candidate, ClientHello, DaemonToPickerMessage, DestinationMetadata,
    OpenRequest, PickerToDaemonMessage, PlacementHint, RealmKind,
};
use d2b_clipd::wayland::{DataControlClient, DataControlSource, HostClipboardEvent};
use rustix::event::{PollFd, PollFlags, poll};
use serde::Deserialize;

const CONTROL_MAX_FRAME_BYTES: usize = 1024;
const BRIDGE_MAX_FRAME_BYTES: usize = 4096;
const BOUNDED_READ_TIMEOUT: Duration = Duration::from_secs(2);
const CURRENT_HOST_ENTRY_ID: &str = "current-host-selection";
const CURRENT_BRIDGE_ENTRY_ID: &str = "current-vm-selection";
const HISTORY_MAX_ENTRIES: usize = 20;
const MATERIALIZE_MAX_BYTES: usize = 8 * 1024 * 1024;
const ACCEPT_RESOURCE_BACKOFF: Duration = Duration::from_millis(50);
const ACCEPT_WARN_INTERVAL: Duration = Duration::from_secs(60);
const STREAM_FRAME_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
    let bridge_peers = parse_bridge_peers(&config_json)?;
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
    let mut audit_queue = AuditQueue::new(AuditQueueConfig {
        per_realm_quota: 1024,
    });
    let mut metrics_queue = MetricsQueue::new(1024);

    // ── Control socket ───────────────────────────────────────────────────────
    let control_socket = control_socket_path()?;
    install_control_socket_parent(&control_socket)?;
    let listener =
        UnixListener::bind(&control_socket).map_err(|e| format!("bind control socket: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;
    let bridge_listeners = install_bridge_listeners(&args.bridge_root, &bridge_peers)?;

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
        control_streams: Vec::new(),
        bridge_listeners,
        bridge_streams: Vec::new(),
        bridge_selection: None,
        history: ClipboardHistory::default(),
        accept_diag: AcceptDiagnostics::default(),
        control_accept_backoff_until: None,
        bridge_accept_backoff_until: None,
        data_control: &mut data_control,
        niri_rx,
        host_clipboard: &mut host_clipboard,
        supervisor: &mut supervisor,
        picker_command,
        fallback: &mut fallback,
        notifier: &mut notifier,
        audit_queue: &mut audit_queue,
        metrics_queue: &mut metrics_queue,
    };
    event_loop.run()
}

fn parse_bridge_peers(config_json: &serde_json::Value) -> Result<Vec<BridgePeerConfig>, String> {
    if let Some(value) = config_json.pointer("/runtime/bridgePeers") {
        let Some(items) = value.as_array() else {
            return Err("runtime.bridgePeers must be an array".to_owned());
        };
        return items
            .iter()
            .map(|item| {
                let vm_name = item
                    .get("vmName")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "runtime.bridgePeers[].vmName must be a string".to_owned())?
                    .to_owned();
                let expected_uid = item
                    .get("expectedUid")
                    .and_then(|value| value.as_u64())
                    .ok_or_else(|| {
                        "runtime.bridgePeers[].expectedUid must be an integer".to_owned()
                    })?;
                Ok(BridgePeerConfig {
                    vm_name,
                    expected_uid: expected_uid
                        .try_into()
                        .map_err(|_| "runtime.bridgePeers[].expectedUid too large".to_owned())?,
                })
            })
            .collect();
    }
    let Some(value) = config_json.pointer("/runtime/bridgeVms") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err("runtime.bridgeVms must be an array".to_owned());
    };
    items
        .iter()
        .map(|item| {
            let vm_name = item
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "runtime.bridgeVms entries must be strings".to_owned())?;
            Ok(BridgePeerConfig {
                vm_name,
                expected_uid: u32::MAX,
            })
        })
        .collect()
}

// ─── Event loop ───────────────────────────────────────────────────────────────

struct EventLoop<'a> {
    listener: &'a UnixListener,
    control_streams: Vec<ControlStream>,
    bridge_listeners: Vec<BridgeListener>,
    bridge_streams: Vec<BridgeStream>,
    bridge_selection: Option<BridgeSelectionState>,
    history: ClipboardHistory,
    accept_diag: AcceptDiagnostics,
    control_accept_backoff_until: Option<Instant>,
    bridge_accept_backoff_until: Option<Instant>,
    data_control: &'a mut DataControlClient,
    niri_rx: mpsc::Receiver<NiriMessage>,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: Option<PickerCommand>,
    fallback: &'a mut FallbackArming,
    notifier: &'a mut DesktopNotifier,
    audit_queue: &'a mut AuditQueue,
    metrics_queue: &'a mut MetricsQueue,
}

impl EventLoop<'_> {
    fn run(&mut self) -> Result<(), String> {
        loop {
            // Flush pending Wayland requests before polling.
            self.data_control.flush().ok();
            let now = Instant::now();
            let control_accept_in_backoff =
                accept_backoff_active(self.control_accept_backoff_until, now);
            if !control_accept_in_backoff {
                self.control_accept_backoff_until = None;
            }
            let bridge_accept_in_backoff =
                accept_backoff_active(self.bridge_accept_backoff_until, now);
            if !bridge_accept_in_backoff {
                self.bridge_accept_backoff_until = None;
            }

            let (
                wayland_ready,
                control_ready,
                picker_ready,
                control_stream_ready,
                bridge_listener_ready,
                bridge_stream_ready,
            ) = {
                let mut poll_fds = vec![
                    PollFd::from_borrowed_fd(
                        self.data_control.as_fd(),
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ),
                    PollFd::new(
                        self.listener,
                        accept_listener_poll_flags(control_accept_in_backoff),
                    ),
                ];
                if let Some(socket) = self.supervisor.active_socket() {
                    poll_fds.push(PollFd::new(
                        socket,
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ));
                }
                for stream in &self.control_streams {
                    poll_fds.push(PollFd::new(
                        &stream.stream,
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ));
                }
                for bridge in &self.bridge_listeners {
                    poll_fds.push(PollFd::new(
                        &bridge.listener,
                        accept_listener_poll_flags(bridge_accept_in_backoff),
                    ));
                }
                for bridge in &self.bridge_streams {
                    poll_fds.push(PollFd::new(
                        &bridge.stream,
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
                    ));
                }
                match poll(&mut poll_fds, self.poll_timeout_ms()) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => return Err(format!("poll failed: {error}")),
                }
                let control_offset = 2 + usize::from(self.supervisor.active_socket().is_some());
                let bridge_listener_offset = control_offset + self.control_streams.len();
                let bridge_stream_offset = bridge_listener_offset + self.bridge_listeners.len();
                let picker_ready = self.supervisor.active_socket().is_some()
                    && poll_fds[2]
                        .revents()
                        .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP);
                let control_stream_ready = poll_fds[control_offset..bridge_listener_offset]
                    .iter()
                    .enumerate()
                    .filter_map(|(index, fd)| {
                        fd.revents()
                            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
                            .then_some(index)
                    })
                    .collect::<Vec<_>>();
                let bridge_listener_ready = poll_fds[bridge_listener_offset..bridge_stream_offset]
                    .iter()
                    .enumerate()
                    .filter_map(|(index, fd)| {
                        fd.revents()
                            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
                            .then_some(index)
                    })
                    .collect::<Vec<_>>();
                let bridge_stream_ready = poll_fds[bridge_stream_offset..]
                    .iter()
                    .enumerate()
                    .filter_map(|(index, fd)| {
                        fd.revents()
                            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
                            .then_some(index)
                    })
                    .collect::<Vec<_>>();
                (
                    poll_fds[0]
                        .revents()
                        .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP),
                    !control_accept_in_backoff
                        && poll_fds[1]
                            .revents()
                            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP),
                    picker_ready,
                    control_stream_ready,
                    if bridge_accept_in_backoff {
                        Vec::new()
                    } else {
                        bridge_listener_ready
                    },
                    bridge_stream_ready,
                )
            };

            drain_niri_channel(&self.niri_rx, self.host_clipboard, self.fallback);

            // ── Wayland events ────────────────────────────────────────────────
            if wayland_ready {
                self.data_control
                    .prepare_and_read()
                    .map_err(|e| format!("wayland read failed: {e}"))?;
            }
            let wl_events = self
                .data_control
                .dispatch_pending()
                .map_err(|e| format!("wayland dispatch failed: {e}"))?;
            for event in wl_events {
                let mut context = WaylandEventContext {
                    data_control: self.data_control,
                    host_clipboard: self.host_clipboard,
                    notifier: self.notifier,
                    fallback: self.fallback,
                    supervisor: self.supervisor,
                    picker_command: &self.picker_command,
                    accept_diag: &mut self.accept_diag,
                    bridge_selection: &mut self.bridge_selection,
                    history: &mut self.history,
                };
                handle_wayland_event(event, &mut context);
            }

            // ── Control socket accepts ────────────────────────────────────────
            if control_ready {
                loop {
                    match self.listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(error) = stream.set_nonblocking(true) {
                                self.accept_diag.warn(
                                    "control",
                                    "stream-nonblocking-failed",
                                    || {
                                        format!(
                                            "d2b-clipd: failed to set control stream nonblocking: {error}"
                                        )
                                    },
                                );
                                continue;
                            }
                            self.control_streams.push(ControlStream {
                                stream,
                                read_buffer: Vec::new(),
                                frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if is_recoverable_accept_error(&error) => {
                            if is_resource_exhaustion_accept_error(&error) {
                                self.control_accept_backoff_until =
                                    Some(Instant::now() + ACCEPT_RESOURCE_BACKOFF);
                            }
                            self.accept_diag
                                .warn("control", "recoverable-accept-error", || {
                                    format!("d2b-clipd: recoverable control accept error: {error}")
                                });
                            break;
                        }
                        Err(error) => return Err(format!("control accept failed: {error}")),
                    }
                }
            }

            for index in control_stream_ready.into_iter().rev() {
                if index < self.control_streams.len() {
                    let mut stream = self.control_streams.swap_remove(index);
                    match handle_control_stream(
                        &mut stream,
                        self.supervisor,
                        &self.picker_command,
                        self.host_clipboard,
                        self.fallback,
                        self.notifier,
                        &mut self.accept_diag,
                    ) {
                        ControlStreamStatus::Done => {}
                        ControlStreamStatus::Incomplete => self.control_streams.push(stream),
                    }
                }
            }

            for index in bridge_listener_ready {
                if let Some(bridge) = self.bridge_listeners.get(index) {
                    let backoff = accept_bridge_streams(
                        bridge,
                        &mut self.bridge_streams,
                        &mut self.accept_diag,
                    );
                    if backoff {
                        self.bridge_accept_backoff_until =
                            Some(Instant::now() + ACCEPT_RESOURCE_BACKOFF);
                    }
                }
            }

            for index in bridge_stream_ready.into_iter().rev() {
                if index < self.bridge_streams.len() {
                    let mut stream = self.bridge_streams.swap_remove(index);
                    let mut context = BridgeHandlerContext {
                        host_clipboard: self.host_clipboard,
                        notifier: self.notifier,
                        data_control: self.data_control,
                        fallback: self.fallback,
                        supervisor: self.supervisor,
                        picker_command: &self.picker_command,
                        accept_diag: &mut self.accept_diag,
                        audit_queue: self.audit_queue,
                        metrics_queue: self.metrics_queue,
                        bridge_selection: &mut self.bridge_selection,
                        history: &mut self.history,
                    };
                    match handle_bridge_stream(&mut stream, &mut context) {
                        BridgeStreamStatus::Done => {}
                        BridgeStreamStatus::Incomplete => self.bridge_streams.push(stream),
                    }
                }
            }

            // ── Picker responses ──────────────────────────────────────────────
            if picker_ready {
                loop {
                    match self
                        .supervisor
                        .poll_active(PICKER_TO_DAEMON_MAX_FRAME_BYTES)
                    {
                        Ok(PickerPoll::Message(message)) => handle_picker_message(
                            message,
                            self.data_control,
                            self.host_clipboard,
                            self.bridge_selection.as_ref(),
                            &self.history,
                            self.notifier,
                            self.fallback,
                            self.supervisor,
                            &mut self.accept_diag,
                        ),
                        Ok(PickerPoll::Closed) => {
                            self.accept_diag
                                .warn("picker", "closed-before-selection", || {
                                    "d2b-clipd: picker exited before completing the paste request"
                                        .to_owned()
                                });
                            let _ = self.fallback.cancel_picker();
                            let _ = self.supervisor.cancel_active(ReasonCode::PickerCrashed);
                            break;
                        }
                        Ok(PickerPoll::Incomplete) => break,
                        Err(error) => {
                            self.accept_diag.warn("picker", "frame-failed", || {
                                format!("d2b-clipd: picker frame failed: {error}")
                            });
                            let _ = self.fallback.cancel_picker();
                            let _ = self.supervisor.cancel_active(ReasonCode::PickerCrashed);
                            break;
                        }
                    }
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
            self.supervisor.reap_terminated(now);
            if let FallbackTransition::Cleared(r) = self.fallback.on_timeout(now) {
                log::debug!("d2b-clipd: fallback armed state cleared: {r:?}");
            }
            self.reap_idle_streams(now);
            self.accept_diag.flush_suppressed();
        }
    }

    fn poll_timeout_ms(&self) -> i32 {
        let now = Instant::now();
        let mut next = self.host_clipboard.pending_paste_deadline();
        if let Some(deadline) = self.supervisor.deadline() {
            next = Some(next.map_or(deadline, |old| old.min(deadline)));
        }
        if let Some(deadline) = self.supervisor.maintenance_deadline() {
            next = Some(next.map_or(deadline, |old| old.min(deadline)));
        }
        if let Some(deadline) = self.control_accept_backoff_until {
            next = Some(next.map_or(deadline, |old| old.min(deadline)));
        }
        if let Some(deadline) = self.bridge_accept_backoff_until {
            next = Some(next.map_or(deadline, |old| old.min(deadline)));
        }
        for stream in &self.control_streams {
            next = Some(next.map_or(stream.frame_deadline, |old| old.min(stream.frame_deadline)));
        }
        for stream in &self.bridge_streams {
            next = Some(next.map_or(stream.frame_deadline, |old| old.min(stream.frame_deadline)));
        }
        if let FallbackState::Armed { expires_at, .. } = self.fallback.state() {
            next = Some(next.map_or(*expires_at, |old| old.min(*expires_at)));
        }
        next.map(|deadline| {
            deadline
                .saturating_duration_since(now)
                .as_millis()
                .min(i32::MAX as u128) as i32
        })
        .unwrap_or(5000)
    }

    fn reap_idle_streams(&mut self, now: Instant) {
        let control_dropped = reap_idle_control_streams(&mut self.control_streams, now);
        if control_dropped > 0 {
            log::debug!("d2b-clipd: reaped {control_dropped} idle control stream(s)");
        }

        let bridge_dropped = reap_idle_bridge_streams(&mut self.bridge_streams, now);
        if bridge_dropped > 0 {
            log::debug!("d2b-clipd: reaped {bridge_dropped} idle bridge stream(s)");
        }
    }
}

fn reap_idle_control_streams(streams: &mut Vec<ControlStream>, now: Instant) -> usize {
    let before = streams.len();
    streams.retain(|stream| stream.frame_deadline > now);
    before.saturating_sub(streams.len())
}

fn reap_idle_bridge_streams(streams: &mut Vec<BridgeStream>, now: Instant) -> usize {
    let before = streams.len();
    streams.retain(|stream| stream.frame_deadline > now);
    before.saturating_sub(streams.len())
}

#[derive(Debug)]
struct BridgePeerConfig {
    vm_name: String,
    expected_uid: u32,
}

#[derive(Debug)]
struct BridgeListener {
    vm_name: String,
    expected_uid: u32,
    listener: UnixListener,
}

#[derive(Debug)]
struct BridgeStream {
    vm_name: String,
    stream: UnixStream,
    read_buffer: Vec<u8>,
    received_fds: Vec<OwnedFd>,
    frame_deadline: Instant,
}

#[derive(Default)]
struct AcceptDiagnostics {
    last_warn: BTreeMap<String, Instant>,
    suppressed: BTreeMap<String, u64>,
}

impl AcceptDiagnostics {
    fn warn(&mut self, scope: &str, reason: &str, message: impl FnOnce() -> String) {
        let key = format!("{scope}:{reason}");
        let now = Instant::now();
        let should_emit = self
            .last_warn
            .get(&key)
            .is_none_or(|last| now.duration_since(*last) >= ACCEPT_WARN_INTERVAL);
        if should_emit {
            if let Some(count) = self.suppressed.remove(&key)
                && count > 0
            {
                log::warn!("d2b-clipd: accept diagnostic suppressed={count} key={key}");
            }
            log::warn!("{}", message());
            self.last_warn.insert(key, now);
        } else {
            *self.suppressed.entry(key).or_insert(0) += 1;
        }
    }

    fn flush_suppressed(&mut self) {
        let now = Instant::now();
        let ready = self
            .last_warn
            .iter()
            .filter_map(|(key, last)| {
                (now.duration_since(*last) >= ACCEPT_WARN_INTERVAL
                    && self.suppressed.get(key).is_some_and(|count| *count > 0))
                .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in ready {
            if let Some(count) = self.suppressed.remove(&key)
                && count > 0
            {
                log::warn!("d2b-clipd: accept diagnostic suppressed={count} key={key}");
            }
            self.last_warn.insert(key, now);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum BridgeFrame {
    VmPasteRequest {
        vm_name: String,
        mime_type: String,
        source_id: u64,
        source_attribution: BridgeAttribution,
    },
    VmCopySelection {
        vm_name: String,
        mime_type: String,
        source_id: u64,
        source_attribution: BridgeAttribution,
    },
}

#[derive(Debug)]
struct BridgeSelectionState {
    vm_name: String,
    vm_source_id: u64,
    data_control_source_id: u64,
    source: Option<DataControlSource>,
    data_by_mime: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone)]
struct ClipboardHistoryEntry {
    entry_id: String,
    source_realm: String,
    source_realm_kind: RealmKind,
    source_app: Option<String>,
    source_app_id: Option<String>,
    source_attribution: AttributionQuality,
    data_by_mime: BTreeMap<String, Vec<u8>>,
    timestamp_unix_ms: u64,
}

#[derive(Debug, Default)]
struct ClipboardHistory {
    entries: VecDeque<ClipboardHistoryEntry>,
    next_id: u64,
}

impl ClipboardHistory {
    fn push(&mut self, mut entry: ClipboardHistoryEntry) {
        if entry.data_by_mime.is_empty() {
            return;
        }
        self.next_id = self.next_id.saturating_add(1);
        entry.entry_id = format!("history-{}", self.next_id);
        self.entries.push_front(entry);
        while self.entries.len() > HISTORY_MAX_ENTRIES {
            self.entries.pop_back();
        }
    }

    fn candidates(&self, requested_mime_type: &str) -> Vec<Candidate> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let bytes = entry.data_by_mime.get(requested_mime_type)?;
                Some(Candidate {
                    entry_id: entry.entry_id.clone(),
                    source_realm: entry.source_realm.clone(),
                    source_realm_kind: entry.source_realm_kind,
                    source_app: entry.source_app.clone(),
                    source_app_id: entry.source_app_id.clone(),
                    source_attribution: entry.source_attribution,
                    preview_text: std::str::from_utf8(bytes)
                        .ok()
                        .map(|text| sanitize_notification_text(text, 256)),
                    content_type: requested_mime_type.to_owned(),
                    timestamp_unix_ms: entry.timestamp_unix_ms,
                    thumbnail_png_base64: None,
                    byte_count: Some(bytes.len() as u64),
                    confirmation_required: false,
                })
            })
            .collect()
    }

    fn bytes_for(&self, entry_id: &str, mime_type: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|entry| entry.entry_id == entry_id)
            .and_then(|entry| entry.data_by_mime.get(mime_type).map(Vec::as_slice))
    }
}

#[derive(Debug)]
enum BridgeRequest {
    Paste(BridgePasteRequest),
    Copy(BridgeCopySelectionRequest),
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum BridgeAttribution {
    ExactClient,
}

fn install_bridge_listeners(
    root: &Path,
    bridge_peers: &[BridgePeerConfig],
) -> Result<Vec<BridgeListener>, String> {
    let uid = rustix::process::getuid().as_raw();
    let mut listeners = Vec::new();
    for peer in bridge_peers {
        let path = bridge_socket_path(root, uid, &peer.vm_name)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("bridge socket has no parent: {}", path.display()))?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o770)
            .create(parent)
            .map_err(|e| format!("create bridge socket dir {}: {e}", parent.display()))?;
        if path.exists() {
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| format!("stat bridge socket {}: {e}", path.display()))?;
            if !meta.file_type().is_socket() {
                return Err(format!("refusing to replace non-socket {}", path.display()));
            }
            std::fs::remove_file(&path)
                .map_err(|e| format!("remove stale bridge socket {}: {e}", path.display()))?;
        }
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("bind bridge socket {}: {e}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))
            .map_err(|e| format!("chmod bridge socket {}: {e}", path.display()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set bridge socket nonblocking {}: {e}", path.display()))?;
        listeners.push(BridgeListener {
            vm_name: peer.vm_name.clone(),
            expected_uid: peer.expected_uid,
            listener,
        });
    }
    Ok(listeners)
}

fn bridge_socket_path(root: &Path, uid: u32, vm: &str) -> Result<PathBuf, String> {
    if vm.is_empty() || vm == "." || vm == ".." || vm.contains('/') || vm.contains('\0') {
        return Err(format!("invalid bridge VM name: {vm:?}"));
    }
    Ok(root
        .join(uid.to_string())
        .join("bridge")
        .join(vm)
        .join("clip.sock"))
}

#[derive(Debug)]
struct BridgePasteRequest {
    vm_name: String,
    mime_type: String,
    source_id: u64,
    fd: OwnedFd,
}

#[derive(Debug)]
struct BridgeCopySelectionRequest {
    vm_name: String,
    mime_type: String,
    source_id: u64,
    fd: OwnedFd,
}

fn accept_bridge_streams(
    bridge: &BridgeListener,
    streams: &mut Vec<BridgeStream>,
    diag: &mut AcceptDiagnostics,
) -> bool {
    let rlimit_nofile = current_nofile_soft_limit();
    if validate_fd_cap(FdCapModel {
        requested_cap: streams.len().saturating_add(1) as u64,
        rlimit_nofile,
        base_reserved: 64,
        max_fds_per_recvmsg: 1,
    })
    .is_err()
    {
        diag.warn("bridge", "stream-cap-exceeded", || {
            format!(
                "d2b-clipd: bridge stream cap exceeded for vm={}",
                bridge.vm_name
            )
        });
        return true;
    }
    loop {
        match bridge.listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = stream.set_nonblocking(true) {
                    diag.warn("bridge", "stream-nonblocking-failed", || {
                        format!(
                            "d2b-clipd: bridge stream nonblocking failed for vm={}: {error}",
                            bridge.vm_name
                        )
                    });
                    continue;
                }
                if let Err(error) = validate_bridge_peer(&stream, bridge.expected_uid) {
                    diag.warn("bridge", "peer-rejected", || {
                        format!(
                            "d2b-clipd: bridge peer rejected for vm={}: {error}",
                            bridge.vm_name
                        )
                    });
                    continue;
                }
                streams.push(BridgeStream {
                    vm_name: bridge.vm_name.clone(),
                    stream,
                    read_buffer: Vec::new(),
                    received_fds: Vec::new(),
                    frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
                });
                if validate_fd_cap(FdCapModel {
                    requested_cap: streams.len().saturating_add(1) as u64,
                    rlimit_nofile,
                    base_reserved: 64,
                    max_fds_per_recvmsg: 1,
                })
                .is_err()
                {
                    return true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if is_recoverable_accept_error(&error) => {
                diag.warn("bridge", "recoverable-accept-error", || {
                    format!(
                        "d2b-clipd: recoverable bridge accept error for vm={}: {error}",
                        bridge.vm_name
                    )
                });
                return is_resource_exhaustion_accept_error(&error);
            }
            Err(error) => {
                diag.warn("bridge", "accept-failed", || {
                    format!(
                        "d2b-clipd: bridge accept failed for vm={}: {error}",
                        bridge.vm_name
                    )
                });
                break;
            }
        }
    }
    false
}

fn is_recoverable_accept_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::Interrupted {
        return true;
    }
    matches!(
        error.raw_os_error(),
        Some(nix::libc::ECONNABORTED | nix::libc::EINTR)
    ) || is_resource_exhaustion_accept_error(error)
}

fn is_resource_exhaustion_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(nix::libc::EMFILE | nix::libc::ENFILE | nix::libc::ENOBUFS | nix::libc::ENOMEM)
    )
}

fn accept_backoff_active(backoff_until: Option<Instant>, now: Instant) -> bool {
    backoff_until.is_some_and(|until| until > now)
}

fn accept_listener_poll_flags(in_backoff: bool) -> PollFlags {
    if in_backoff {
        PollFlags::empty()
    } else {
        PollFlags::IN | PollFlags::ERR | PollFlags::HUP
    }
}

fn current_nofile_soft_limit() -> u64 {
    nix::sys::resource::getrlimit(nix::sys::resource::Resource::RLIMIT_NOFILE)
        .map(|(soft, _)| soft)
        .unwrap_or(1024)
}

enum BridgeStreamStatus {
    Done,
    Incomplete,
}

struct BridgeHandlerContext<'a> {
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    notifier: &'a mut DesktopNotifier,
    data_control: &'a mut DataControlClient,
    fallback: &'a mut FallbackArming,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &'a Option<PickerCommand>,
    accept_diag: &'a mut AcceptDiagnostics,
    audit_queue: &'a mut AuditQueue,
    metrics_queue: &'a mut MetricsQueue,
    bridge_selection: &'a mut Option<BridgeSelectionState>,
    history: &'a mut ClipboardHistory,
}

fn handle_bridge_stream(
    bridge: &mut BridgeStream,
    context: &mut BridgeHandlerContext<'_>,
) -> BridgeStreamStatus {
    match recv_bridge_frame(bridge) {
        Ok(request) => {
            handle_bridge_request(request, context);
            loop {
                match recv_bridge_frame(bridge) {
                    Ok(request) => handle_bridge_request(request, context),
                    Err(BridgeReadError::Incomplete) => return BridgeStreamStatus::Incomplete,
                    Err(error) => {
                        context.accept_diag.warn("bridge", "frame-failed", || {
                            format!(
                                "d2b-clipd: bridge frame failed for vm={}: {}",
                                bridge.vm_name,
                                error.message()
                            )
                        });
                        return BridgeStreamStatus::Done;
                    }
                }
            }
        }
        Err(BridgeReadError::Incomplete) => return BridgeStreamStatus::Incomplete,
        Err(error) => {
            context.accept_diag.warn("bridge", "frame-failed", || {
                format!(
                    "d2b-clipd: bridge frame failed for vm={}: {}",
                    bridge.vm_name,
                    error.message()
                )
            });
        }
    }
    BridgeStreamStatus::Done
}

fn validate_bridge_peer(stream: &UnixStream, expected_uid: u32) -> Result<(), String> {
    let creds = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map_err(|e| e.to_string())?;
    if creds.uid() == expected_uid {
        Ok(())
    } else {
        Err(format!(
            "uid mismatch: expected {}, got {}",
            expected_uid,
            creds.uid()
        ))
    }
}

#[derive(Debug)]
enum BridgeReadError {
    Incomplete,
    Invalid(String),
}

impl BridgeReadError {
    fn message(&self) -> &str {
        match self {
            Self::Incomplete => "incomplete bridge frame",
            Self::Invalid(message) => message,
        }
    }
}

fn recv_bridge_frame(stream: &mut BridgeStream) -> Result<BridgeRequest, BridgeReadError> {
    loop {
        if stream.read_buffer.contains(&b'\n') {
            return parse_bridge_frame(stream);
        }
        let mut buf = [0_u8; 4096];
        let mut iov = [std::io::IoSliceMut::new(&mut buf)];
        let mut cmsg_space = [0_u8; rustix::cmsg_space!(ScmRights(64))];
        let mut control = rustix::net::RecvAncillaryBuffer::new(&mut cmsg_space);
        let msg = match rustix::net::recvmsg(
            &stream.stream,
            &mut iov,
            &mut control,
            rustix::net::RecvFlags::DONTWAIT | rustix::net::RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(msg) => msg,
            Err(rustix::io::Errno::AGAIN) => {
                return if stream.read_buffer.contains(&b'\n') {
                    parse_bridge_frame(stream)
                } else {
                    Err(BridgeReadError::Incomplete)
                };
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(BridgeReadError::Invalid(error.to_string())),
        };
        for cmsg in control.drain() {
            if let rustix::net::RecvAncillaryMessage::ScmRights(fds) = cmsg {
                stream.received_fds.extend(fds);
            }
        }
        validate_bridge_fd_queue(stream, msg.flags)?;
        if msg.bytes == 0 {
            return Err(BridgeReadError::Invalid(
                "bridge stream closed before complete frame".to_owned(),
            ));
        }
        stream.read_buffer.extend_from_slice(&buf[..msg.bytes]);
        let queued_frames = stream
            .read_buffer
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        let has_partial_followup = stream.read_buffer.last().is_some_and(|byte| *byte != b'\n');
        let max_expected_fds = queued_frames + usize::from(has_partial_followup);
        if max_expected_fds > 0 && stream.received_fds.len() > max_expected_fds {
            stream.received_fds.clear();
            return Err(BridgeReadError::Invalid(
                "bridge frame carried too many transfer fds".to_owned(),
            ));
        }
        if stream.read_buffer.len() > BRIDGE_MAX_FRAME_BYTES {
            stream.received_fds.clear();
            return Err(BridgeReadError::Invalid(
                "bridge frame too large".to_owned(),
            ));
        }
        if stream.read_buffer.contains(&b'\n') {
            return parse_bridge_frame(stream);
        }
    }
}

fn validate_bridge_fd_queue(
    stream: &mut BridgeStream,
    flags: rustix::net::RecvFlags,
) -> Result<(), BridgeReadError> {
    if stream.received_fds.len() > 64 {
        stream.received_fds.clear();
        return Err(BridgeReadError::Invalid(
            "bridge frame carried too many queued transfer fds".to_owned(),
        ));
    }
    let control_truncated = rustix::net::RecvFlags::from_bits_retain(nix::libc::MSG_CTRUNC as u32);
    if flags.contains(control_truncated) {
        stream.received_fds.clear();
        return Err(BridgeReadError::Invalid(
            "bridge control message truncated".to_owned(),
        ));
    }
    Ok(())
}

fn parse_bridge_frame(stream: &mut BridgeStream) -> Result<BridgeRequest, BridgeReadError> {
    let newline = stream
        .read_buffer
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(BridgeReadError::Incomplete)?;
    let frame_bytes = stream.read_buffer.drain(..=newline).collect::<Vec<_>>();
    let frame: BridgeFrame = serde_json::from_slice(&frame_bytes)
        .map_err(|e| BridgeReadError::Invalid(e.to_string()))?;
    match frame {
        BridgeFrame::VmPasteRequest {
            vm_name,
            mime_type,
            source_id,
            source_attribution,
        } => parse_bridge_transfer(
            stream,
            vm_name,
            mime_type,
            source_id,
            source_attribution,
            false,
        ),
        BridgeFrame::VmCopySelection {
            vm_name,
            mime_type,
            source_id,
            source_attribution,
        } => parse_bridge_transfer(
            stream,
            vm_name,
            mime_type,
            source_id,
            source_attribution,
            true,
        ),
    }
}

fn parse_bridge_transfer(
    stream: &mut BridgeStream,
    vm_name: String,
    mime_type: String,
    source_id: u64,
    source_attribution: BridgeAttribution,
    copy_selection: bool,
) -> Result<BridgeRequest, BridgeReadError> {
    if stream.received_fds.is_empty() {
        return Err(BridgeReadError::Invalid(
            "bridge frame did not include transfer fd".to_owned(),
        ));
    };
    let fd = stream.received_fds.remove(0);
    if vm_name != stream.vm_name {
        drop(fd);
        return Err(BridgeReadError::Invalid(format!(
            "bridge vm mismatch: expected {}, got {vm_name}",
            stream.vm_name
        )));
    }
    if source_attribution != BridgeAttribution::ExactClient {
        drop(fd);
        return Err(BridgeReadError::Invalid(
            "bridge frame did not carry exact attribution".to_owned(),
        ));
    }
    if let Err(error) = classify_fd(&fd) {
        drop(fd);
        return Err(BridgeReadError::Invalid(format!(
            "bridge transfer fd rejected: {error}"
        )));
    }
    log::debug!(
        "d2b-clipd: received VM bridge paste request vm={} source_id={} mime={}",
        bounded_label(&stream.vm_name),
        source_id,
        bounded_mime(&mime_type)
    );
    stream.frame_deadline = Instant::now() + STREAM_FRAME_IDLE_TIMEOUT;
    if copy_selection {
        Ok(BridgeRequest::Copy(BridgeCopySelectionRequest {
            vm_name,
            mime_type,
            source_id,
            fd,
        }))
    } else {
        Ok(BridgeRequest::Paste(BridgePasteRequest {
            vm_name,
            mime_type,
            source_id,
            fd,
        }))
    }
}

fn handle_bridge_request(request: BridgeRequest, context: &mut BridgeHandlerContext<'_>) {
    match request {
        BridgeRequest::Paste(request) => handle_bridge_paste_request(request, context),
        BridgeRequest::Copy(request) => handle_bridge_copy_selection(request, context),
    }
}

fn handle_bridge_copy_selection(
    request: BridgeCopySelectionRequest,
    context: &mut BridgeHandlerContext<'_>,
) {
    let BridgeCopySelectionRequest {
        vm_name,
        mime_type,
        source_id,
        fd,
    } = request;
    if !is_mime_allowed(&mime_type) {
        drop(fd);
        return;
    }
    let bytes = match read_fd_to_vec(fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT) {
        Ok(bytes) => bytes,
        Err(reason) => {
            context
                .accept_diag
                .warn("bridge", "copy-materialize-failed", || {
                    format!(
                        "d2b-clipd: bridge copy materialize failed for vm={}: {}",
                        bounded_label(&vm_name),
                        reason.as_str()
                    )
                });
            return;
        }
    };
    log::info!(
        "d2b-clipd: bridge copy received vm={} source_id={} mime={} bytes={}",
        bounded_label(&vm_name),
        source_id,
        bounded_mime(&mime_type),
        bytes.len()
    );

    let replace = context.bridge_selection.as_ref().is_none_or(|selection| {
        selection.vm_name != vm_name || selection.vm_source_id != source_id
    });
    if replace {
        *context.bridge_selection = Some(BridgeSelectionState {
            vm_name: vm_name.clone(),
            vm_source_id: source_id,
            data_control_source_id: 0,
            source: None,
            data_by_mime: BTreeMap::new(),
        });
    }

    let Some(selection) = context.bridge_selection.as_mut() else {
        return;
    };
    selection.data_by_mime.insert(mime_type, bytes);
    context.history.push(ClipboardHistoryEntry {
        entry_id: String::new(),
        source_realm: selection.vm_name.clone(),
        source_realm_kind: RealmKind::Vm,
        source_app: Some(format!("{} VM", selection.vm_name)),
        source_app_id: Some(format!("d2b.{}", selection.vm_name)),
        source_attribution: AttributionQuality::ExactClient,
        data_by_mime: selection.data_by_mime.clone(),
        timestamp_unix_ms: unix_millis(),
    });
    let mimes = selection.data_by_mime.keys().cloned().collect::<Vec<_>>();
    match context.data_control.create_source(&mimes) {
        Ok((source, source_id)) => {
            if let Err(error) = context.data_control.set_selection(&source) {
                context
                    .accept_diag
                    .warn("bridge", "copy-set-selection-failed", || {
                        format!(
                            "d2b-clipd: bridge copy set selection failed for vm={}: {error}",
                            bounded_label(&selection.vm_name)
                        )
                    });
                return;
            }
            if let Err(error) = context.data_control.flush() {
                context.accept_diag.warn("bridge", "copy-flush-failed", || {
                    format!(
                        "d2b-clipd: bridge copy flush failed for vm={}: {error}",
                        bounded_label(&selection.vm_name)
                    )
                });
                return;
            }
            selection.source = Some(source);
            selection.data_control_source_id = source_id;
            log::debug!(
                "d2b-clipd: bridge copy selection published vm={} mimes={}",
                bounded_label(&selection.vm_name),
                selection.data_by_mime.len()
            );
        }
        Err(error) => {
            context
                .accept_diag
                .warn("bridge", "copy-source-create-failed", || {
                    format!(
                        "d2b-clipd: bridge copy source create failed for vm={}: {error}",
                        bounded_label(&selection.vm_name)
                    )
                });
        }
    }
}

fn handle_bridge_paste_request(
    request: BridgePasteRequest,
    context: &mut BridgeHandlerContext<'_>,
) {
    let BridgePasteRequest {
        vm_name,
        mime_type,
        source_id,
        fd,
    } = request;
    let request_id = format!("bridge-{vm_name}-{source_id}");
    if !is_mime_allowed(&mime_type) {
        drop(fd);
        context.metrics_queue.enqueue_droppable(MetricEvent {
            name: MetricName::PolicyDenied,
            reason: Some(ReasonCode::MimeRejected),
        });
        d2b_clipd::notifications::emit_user_visible_failure(
            context.notifier,
            ReasonCode::MimeRejected,
            "host",
            &vm_name,
        );
        let _ = context.audit_queue.enqueue_fail_closed(AuditEvent {
            request_id,
            source_realm: "host".to_owned(),
            destination_realm: vm_name,
            mime_type,
            byte_count: 0,
            decision: AuditDecision::Deny,
            attribution: d2b_clipd::policy::AttributionQuality::ExactClient,
            reason: ReasonCode::MimeRejected,
            timestamp_unix_ms: unix_millis(),
        });
        let _ = context.audit_queue.drain_all();
        return;
    }
    let dest = context
        .host_clipboard
        .refresh_focused_window_snapshot()
        .unwrap_or_else(|| FocusedWindowSnapshot {
            id: None,
            app_id: Some(format!("d2b.{vm_name}")),
            title: Some(format!("{vm_name} VM")),
            workspace_id: None,
            output_label: None,
        });
    if let Err(reason) = context.audit_queue.enqueue_fail_closed(AuditEvent {
        request_id: request_id.clone(),
        source_realm: "host".to_owned(),
        destination_realm: vm_name.clone(),
        mime_type: mime_type.clone(),
        byte_count: 0,
        decision: AuditDecision::Allow,
        attribution: d2b_clipd::policy::AttributionQuality::ExactClient,
        reason: ReasonCode::Allowed,
        timestamp_unix_ms: unix_millis(),
    }) {
        drop(fd);
        context.metrics_queue.enqueue_droppable(MetricEvent {
            name: MetricName::AuditQueueOverflow,
            reason: Some(reason),
        });
        context
            .accept_diag
            .warn("bridge", "audit-queue-failed", || {
                format!(
                    "d2b-clipd: bridge audit queue failed for vm={}: {}",
                    bounded_label(&vm_name),
                    reason.as_str()
                )
            });
        return;
    }
    let _ = context.audit_queue.drain_all();
    match context
        .host_clipboard
        .accept_paste_fd_for_destination(fd, mime_type, dest.clone())
    {
        Ok(dest) => {
            if !fulfill_armed_fallback(
                context.fallback,
                context.host_clipboard,
                context.data_control,
                context.notifier,
            ) {
                open_picker_or_arm_fallback(
                    context.fallback,
                    dest,
                    context.host_clipboard,
                    context.notifier,
                    context.supervisor,
                    context.picker_command,
                    context.accept_diag,
                    context.history,
                );
            }
        }
        Err(reason) => {
            d2b_clipd::notifications::emit_user_visible_failure(
                context.notifier,
                reason,
                "host",
                &vm_name,
            );
        }
    }
}

// ─── Wayland event handler ────────────────────────────────────────────────────

struct WaylandEventContext<'a> {
    data_control: &'a mut DataControlClient,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    notifier: &'a mut DesktopNotifier,
    fallback: &'a mut FallbackArming,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &'a Option<PickerCommand>,
    accept_diag: &'a mut AcceptDiagnostics,
    bridge_selection: &'a mut Option<BridgeSelectionState>,
    history: &'a mut ClipboardHistory,
}

fn handle_wayland_event(event: HostClipboardEvent, context: &mut WaylandEventContext<'_>) {
    match event {
        HostClipboardEvent::SelectionChanged {
            offer,
            allowed_mimes,
            has_secret,
        } => {
            if let Some(offer_ref) = offer.as_ref() {
                let data_by_mime =
                    materialize_offer_mimes(context.data_control, offer_ref, &allowed_mimes);
                if !data_by_mime.is_empty() {
                    let window = context.host_clipboard.focused_window_snapshot();
                    context.history.push(ClipboardHistoryEntry {
                        entry_id: String::new(),
                        source_realm: "Host".to_owned(),
                        source_realm_kind: RealmKind::Host,
                        source_app: window
                            .as_ref()
                            .and_then(|window| window.title.clone())
                            .or_else(|| Some("Host clipboard".to_owned())),
                        source_app_id: window.as_ref().and_then(|window| window.app_id.clone()),
                        source_attribution: AttributionQuality::FocusedWindowGuess,
                        data_by_mime,
                        timestamp_unix_ms: unix_millis(),
                    });
                }
            }
            // A new native selection supersedes any armed fallback.
            let _ = context.fallback.on_native_selection_changed();
            context
                .host_clipboard
                .on_host_selection_changed(offer, allowed_mimes, has_secret);
        }
        HostClipboardEvent::SelectionCleared => {
            context.host_clipboard.on_host_selection_cleared();
        }
        HostClipboardEvent::SourceSendRequest {
            source_id,
            mime_type,
            fd,
        } => {
            if let Some(selection) = context.bridge_selection.as_ref()
                && selection.data_control_source_id == source_id
            {
                if !selection.data_by_mime.contains_key(&mime_type) {
                    log::warn!(
                        "d2b-clipd: bridge selection missing requested mime={}",
                        bounded_mime(&mime_type)
                    );
                    drop(fd);
                    return;
                }
                let dest = context
                    .host_clipboard
                    .refresh_focused_window_snapshot()
                    .unwrap_or_default();
                if context.host_clipboard.pending_paste().is_some() {
                    context.host_clipboard.queue_paste_fd_for_destination(
                        fd,
                        mime_type.clone(),
                        dest,
                    );
                    return;
                }
                match context.host_clipboard.accept_paste_fd_for_destination(
                    fd,
                    mime_type.clone(),
                    dest.clone(),
                ) {
                    Ok(dest) => {
                        let candidates = picker_bridge_candidates(selection, &mime_type);
                        open_picker_for_candidates(
                            context.fallback,
                            dest,
                            context.host_clipboard,
                            context.notifier,
                            context.supervisor,
                            context.picker_command,
                            context.accept_diag,
                            candidates,
                        );
                    }
                    Err(reason) => {
                        d2b_clipd::notifications::emit_user_visible_failure(
                            context.notifier,
                            reason,
                            &selection.vm_name,
                            "host",
                        );
                    }
                }
                return;
            }
            // Host application requesting paste data.  Hold the write FD.
            if let Some(existing) = context.host_clipboard.pending_paste() {
                context.host_clipboard.queue_paste_fd_for_destination(
                    fd,
                    mime_type.clone(),
                    existing.destination.clone(),
                );
                return;
            }
            match context
                .host_clipboard
                .accept_paste_fd(fd, mime_type.clone())
            {
                Ok(dest) => {
                    log::debug!(
                        "d2b-clipd: paste fd held for mime={} dest={}",
                        bounded_mime(&mime_type),
                        bounded_label(dest.app_id.as_deref().unwrap_or("unknown"))
                    );
                    if !fulfill_armed_fallback(
                        context.fallback,
                        context.host_clipboard,
                        context.data_control,
                        context.notifier,
                    ) {
                        open_picker_or_arm_fallback(
                            context.fallback,
                            dest,
                            context.host_clipboard,
                            context.notifier,
                            context.supervisor,
                            context.picker_command,
                            context.accept_diag,
                            context.history,
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

struct ControlStream {
    stream: UnixStream,
    read_buffer: Vec<u8>,
    frame_deadline: Instant,
}

enum ControlStreamStatus {
    Done,
    Incomplete,
}

fn handle_control_stream(
    control: &mut ControlStream,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
    notifier: &mut DesktopNotifier,
    accept_diag: &mut AcceptDiagnostics,
) -> ControlStreamStatus {
    match read_control_command_from_stream(control) {
        Ok(ControlCommand::Arm) => {
            let response = handle_arm(
                supervisor,
                picker_command,
                host_clipboard,
                fallback,
                notifier,
                accept_diag,
            );
            let body = match response {
                Ok(msg) => format!("{{\"ok\":true,\"message\":{}}}\n", json_string(&msg)),
                Err(err) => format!("{{\"ok\":false,\"error\":{}}}\n", json_string(&err)),
            };
            if let Err(error) =
                write_all_nonblocking_stream(&control.stream, body.as_bytes(), BOUNDED_READ_TIMEOUT)
            {
                log::warn!("d2b-clipd: write control response failed: {error}");
            }
            ControlStreamStatus::Done
        }
        Err(ControlReadError::Incomplete) => ControlStreamStatus::Incomplete,
        Err(ControlReadError::Closed) => ControlStreamStatus::Done,
        Err(ControlReadError::Invalid(error)) => {
            let body = format!("{{\"ok\":false,\"error\":{}}}\n", json_string(&error));
            if let Err(error) =
                write_all_nonblocking_stream(&control.stream, body.as_bytes(), BOUNDED_READ_TIMEOUT)
            {
                log::warn!("d2b-clipd: write control error response failed: {error}");
            }
            ControlStreamStatus::Done
        }
    }
}

fn write_all_nonblocking_stream(
    stream: &UnixStream,
    data: &[u8],
    timeout: Duration,
) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut remaining = data;
    while !remaining.is_empty() {
        match rustix::io::write(stream, remaining) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "control socket write returned zero",
                ));
            }
            Ok(written) => remaining = &remaining[written..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "control socket write timed out",
                    ));
                }
                let timeout_ms = deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                let mut fds = [PollFd::new(
                    stream,
                    PollFlags::OUT | PollFlags::ERR | PollFlags::HUP,
                )];
                match poll(&mut fds, timeout_ms) {
                    Ok(0) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "control socket write timed out",
                        ));
                    }
                    Err(rustix::io::Errno::INTR) => {}
                    Ok(_) if fds[0].revents().intersects(PollFlags::ERR | PollFlags::HUP) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "control socket closed while writing",
                        ));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return Err(std::io::Error::from_raw_os_error(error.raw_os_error()));
                    }
                }
            }
            Err(error) => return Err(std::io::Error::from_raw_os_error(error.raw_os_error())),
        }
    }
    Ok(())
}

#[derive(Debug)]
enum ControlReadError {
    Closed,
    Incomplete,
    Invalid(String),
}

fn read_control_command_from_stream(
    control: &mut ControlStream,
) -> Result<ControlCommand, ControlReadError> {
    loop {
        let mut buf = [0_u8; 256];
        match control.stream.read(&mut buf) {
            Ok(0) if control.read_buffer.is_empty() => {
                return Err(ControlReadError::Closed);
            }
            Ok(0) => {
                return Err(ControlReadError::Invalid(
                    "peer closed with incomplete control frame".to_owned(),
                ));
            }
            Ok(n) => {
                control.read_buffer.extend_from_slice(&buf[..n]);
                if control.read_buffer.len() > CONTROL_MAX_FRAME_BYTES {
                    return Err(ControlReadError::Invalid(format!(
                        "frame exceeds {CONTROL_MAX_FRAME_BYTES} bytes"
                    )));
                }
                if let Some(newline) = control.read_buffer.iter().position(|byte| *byte == b'\n') {
                    let frame = control.read_buffer.drain(..=newline).collect::<Vec<_>>();
                    return match serde_json::from_slice::<ControlFrame>(&frame)
                        .map_err(|e| format!("invalid control JSON: {e}"))
                    {
                        Ok(ControlFrame::Arm) => Ok(ControlCommand::Arm),
                        Err(error) => Err(ControlReadError::Invalid(error)),
                    };
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(ControlReadError::Incomplete);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ControlReadError::Invalid(error.to_string())),
        }
    }
}

fn handle_picker_message(
    message: PickerToDaemonMessage,
    data_control: &mut DataControlClient,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    bridge_selection: Option<&BridgeSelectionState>,
    history: &ClipboardHistory,
    notifier: &mut DesktopNotifier,
    fallback: &mut FallbackArming,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    _accept_diag: &mut AcceptDiagnostics,
) {
    match message {
        PickerToDaemonMessage::Select(select) => {
            log::debug!(
                "d2b-clipd: picker selected entry for request {}",
                select.request_id
            );
            if host_clipboard.pending_paste().is_some() {
                match materialize_selected_entry_fd(
                    host_clipboard,
                    data_control,
                    bridge_selection,
                    history,
                    &select.entry_id,
                )
                .and_then(|read_fd| spawn_materialize_to_pending_paste(host_clipboard, read_fd))
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
        target,
        expires_at,
    } = fallback.state().clone()
    else {
        return false;
    };
    if Instant::now() >= expires_at {
        let _ = fallback.on_timeout(Instant::now());
        return false;
    }
    if host_clipboard.pending_paste().is_none() {
        return false;
    }
    if reject_background_probe_if_target_mismatch(&target, host_clipboard) {
        return true;
    }
    let result = materialize_selected_entry_fd(
        host_clipboard,
        data_control,
        None,
        &ClipboardHistory::default(),
        &entry_id,
    )
        .and_then(|read_fd| spawn_materialize_to_pending_paste(host_clipboard, read_fd));
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

fn reject_background_probe_if_target_mismatch(
    target: &FocusedWindowSnapshot,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
) -> bool {
    let Some(paste) = host_clipboard.pending_paste() else {
        return false;
    };
    if target.same_target(&paste.destination) {
        return false;
    }
    if let Some(paste) = host_clipboard.take_pending_paste() {
        paste.close_with_reason(ReasonCode::BackgroundProbe);
    }
    true
}

fn materialize_selected_entry_fd(
    host_clipboard: &HostClipboard<NiriQueryProvider>,
    data_control: &mut DataControlClient,
    bridge_selection: Option<&BridgeSelectionState>,
    history: &ClipboardHistory,
    entry_id: &str,
) -> Result<std::os::fd::OwnedFd, ReasonCode> {
    let requested_mime = host_clipboard
        .pending_paste()
        .map(|paste| paste.mime_type.as_str())
        .ok_or(ReasonCode::IntentMissing)?;
    if let Some(bytes) = history.bytes_for(entry_id, requested_mime) {
        let (read_fd, write_fd) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
            .map_err(|_| ReasonCode::FdClosed)?;
        write_all_nonblocking_fd(&write_fd, bytes, Instant::now() + BOUNDED_READ_TIMEOUT)?;
        drop(write_fd);
        return Ok(read_fd);
    }
    if entry_id == CURRENT_BRIDGE_ENTRY_ID {
        let selection = bridge_selection.ok_or(ReasonCode::RequestExpired)?;
        let bytes = selection
            .data_by_mime
            .get(requested_mime)
            .ok_or(ReasonCode::MimeRejected)?;
        let (read_fd, write_fd) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
            .map_err(|_| ReasonCode::FdClosed)?;
        write_all_nonblocking_fd(&write_fd, bytes, Instant::now() + BOUNDED_READ_TIMEOUT)?;
        drop(write_fd);
        return Ok(read_fd);
    }
    if entry_id != CURRENT_HOST_ENTRY_ID {
        return Err(ReasonCode::PolicyDenied);
    }
    let selection = host_clipboard
        .current_selection()
        .ok_or(ReasonCode::RequestExpired)?;
    let offer = selection.offer.as_ref().ok_or(ReasonCode::PolicyDenied)?;
    if !selection
        .allowed_mimes
        .iter()
        .any(|mime| mime == requested_mime)
    {
        return Err(ReasonCode::MimeRejected);
    }
    let (read_fd, write_fd) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        .map_err(|_| ReasonCode::FdClosed)?;
    offer.receive(requested_mime.to_owned(), &write_fd);
    data_control
        .flush()
        .map_err(|_| ReasonCode::BridgeUnavailable)?;
    drop(write_fd);
    Ok(read_fd)
}

fn materialize_offer_mimes(
    data_control: &mut DataControlClient,
    offer: &d2b_clipd::wayland::DataControlOffer,
    allowed_mimes: &[String],
) -> BTreeMap<String, Vec<u8>> {
    let mut data_by_mime = BTreeMap::new();
    for mime in allowed_mimes {
        let Ok((read_fd, write_fd)) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        else {
            continue;
        };
        offer.receive(mime.clone(), &write_fd);
        if data_control.flush().is_err() {
            drop(write_fd);
            drop(read_fd);
            continue;
        }
        drop(write_fd);
        if let Ok(bytes) = read_fd_to_vec(read_fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT) {
            data_by_mime.insert(mime.clone(), bytes);
        }
    }
    data_by_mime
}

fn spawn_materialize_to_pending_paste(
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    read_fd: std::os::fd::OwnedFd,
) -> Result<(), ReasonCode> {
    let paste = host_clipboard
        .take_pending_paste()
        .ok_or(ReasonCode::IntentMissing)?;
    let mime = paste.mime_type.clone();
    let deadline = paste.deadline;
    std::thread::Builder::new()
        .name("d2b-clipd-materialize-write".to_owned())
        .spawn(move || {
            match read_fd_to_vec(read_fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT)
                .and_then(|bytes| write_all_nonblocking_fd(&paste.fd, &bytes, deadline))
            {
                Ok(()) => {
                    log::debug!("d2b-clipd: materialized paste write complete");
                }
                Err(reason) => {
                    log::debug!(
                        "d2b-clipd: materialized paste write failed for mime={}: {}",
                        bounded_mime(&mime),
                        reason.as_str()
                    );
                }
            }
            drop(paste.fd);
        })
        .map_err(|_| ReasonCode::FdWriteTimeout)?;
    Ok(())
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

fn write_all_nonblocking_fd(
    fd: &std::os::fd::OwnedFd,
    data: &[u8],
    deadline: Instant,
) -> Result<(), ReasonCode> {
    use rustix::event::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;

    rustix::io::ioctl_fionbio(fd.as_fd(), true).map_err(|_| ReasonCode::FdClosed)?;
    let mut remaining = data;
    while !remaining.is_empty() {
        match rustix::io::write(fd, remaining) {
            Ok(0) => return Err(ReasonCode::FdClosed),
            Ok(written) => remaining = &remaining[written..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(ReasonCode::FdWriteTimeout);
                }
                let timeout = deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                let mut fds = [PollFd::new(
                    fd,
                    PollFlags::OUT | PollFlags::ERR | PollFlags::HUP,
                )];
                match poll(&mut fds, timeout) {
                    Ok(0) => return Err(ReasonCode::FdWriteTimeout),
                    Ok(_) if fds[0].revents().intersects(PollFlags::ERR | PollFlags::HUP) => {
                        return Err(ReasonCode::FdClosed);
                    }
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => {}
                    Err(_) => return Err(ReasonCode::FdClosed),
                }
            }
            Err(_) => return Err(ReasonCode::FdClosed),
        }
    }
    Ok(())
}

/// `d2b clipboard arm` sends `{"type":"arm"}` to this socket.
/// We open the picker (if configured) and arm the native-paste fallback for
/// the current focused window.
fn handle_arm(
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
    notifier: &mut DesktopNotifier,
    accept_diag: &mut AcceptDiagnostics,
) -> Result<String, String> {
    let dest = host_clipboard
        .refresh_focused_window_snapshot()
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
            let requested_mime = host_clipboard
                .pending_paste()
                .map(|paste| paste.mime_type.clone())
                .unwrap_or_else(|| "text/plain".to_owned());
            let empty_history = ClipboardHistory::default();
            let candidates = picker_candidates(host_clipboard, &empty_history, &requested_mime);
            match picker_handshake(socket, &request_id, &dest, &requested_mime, candidates) {
                Ok(picker_version) => {
                    log::debug!("d2b-clipd: picker opened (version={picker_version})");
                    Ok("picker opened".to_owned())
                }
                Err(error) => {
                    accept_diag.warn("picker", "handshake-failed", || {
                        format!("d2b-clipd: picker handshake failed: {error}")
                    });
                    let _ = supervisor.cancel_active(ReasonCode::PickerCrashed);
                    let _ = fallback.cancel_picker();
                    arm_native_fallback(fallback, dest.clone(), host_clipboard, notifier);
                    Ok("fallback armed".to_owned())
                }
            }
        }
        Err(e) => {
            accept_diag.warn("picker", "launch-failed", || {
                format!("d2b-clipd: picker launch failed: {e}; arming native fallback")
            });
            let _ = fallback.cancel_picker();
            arm_native_fallback(fallback, dest.clone(), host_clipboard, notifier);
            Ok("fallback armed".to_owned())
        }
    }
}

/// Perform the picker ClientHello / OpenRequest handshake.
/// Returns the picker version string on success.
fn picker_handshake(
    socket: &UnixStream,
    request_id: &str,
    dest: &FocusedWindowSnapshot,
    requested_mime_type: &str,
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
        requested_mime_type: requested_mime_type.to_owned(),
        expires_at_unix_ms: unix_millis().saturating_add(30_000),
        placement_hints: Some(PlacementHint {
            pointer_x: None,
            pointer_y: None,
            output_width: None,
            output_height: None,
            overlay_width: Some(420),
            overlay_height: Some(520),
            output: dest.output_label.clone(),
        }),
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

fn picker_candidates(
    host_clipboard: &HostClipboard<NiriQueryProvider>,
    history: &ClipboardHistory,
    requested_mime_type: &str,
) -> Vec<Candidate> {
    let mut candidates = history.candidates(requested_mime_type);
    let Some(selection) = host_clipboard.current_selection() else {
        return candidates;
    };
    if selection.offer.is_none() || selection.allowed_mimes.is_empty() {
        return candidates;
    }
    if !selection
        .allowed_mimes
        .iter()
        .any(|mime| mime == requested_mime_type)
    {
        return candidates;
    }
    let window = selection.attribution.window.as_ref();
    candidates.insert(0, Candidate {
        entry_id: CURRENT_HOST_ENTRY_ID.to_owned(),
        source_realm: "Host".to_owned(),
        source_realm_kind: RealmKind::Host,
        source_app: window
            .and_then(|window| window.title.clone())
            .or_else(|| Some("Host clipboard".to_owned())),
        source_app_id: window.and_then(|window| window.app_id.clone()),
        source_attribution: protocol_attribution(selection.attribution.quality),
        preview_text: None,
        content_type: requested_mime_type.to_owned(),
        timestamp_unix_ms: unix_millis(),
        thumbnail_png_base64: None,
        byte_count: None,
        confirmation_required: false,
    });
    candidates
}

fn picker_bridge_candidates(
    selection: &BridgeSelectionState,
    requested_mime_type: &str,
) -> Vec<Candidate> {
    if !selection.data_by_mime.contains_key(requested_mime_type) {
        return Vec::new();
    }
    vec![Candidate {
        entry_id: CURRENT_BRIDGE_ENTRY_ID.to_owned(),
        source_realm: selection.vm_name.clone(),
        source_realm_kind: RealmKind::Vm,
        source_app: Some(format!("{} VM", selection.vm_name)),
        source_app_id: Some(format!("d2b.{}", selection.vm_name)),
        source_attribution: AttributionQuality::ExactClient,
        preview_text: selection
            .data_by_mime
            .get(requested_mime_type)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(|text| sanitize_notification_text(text, 256)),
        content_type: requested_mime_type.to_owned(),
        timestamp_unix_ms: unix_millis(),
        thumbnail_png_base64: None,
        byte_count: selection
            .data_by_mime
            .get(requested_mime_type)
            .map(|bytes| bytes.len() as u64),
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
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut impl Notifier,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    accept_diag: &mut AcceptDiagnostics,
    history: &ClipboardHistory,
) {
    let requested_mime = host_clipboard
        .pending_paste()
        .map(|paste| paste.mime_type.clone())
        .unwrap_or_else(|| "text/plain".to_owned());
    let candidates = picker_candidates(host_clipboard, history, &requested_mime);
    open_picker_for_candidates(
        fallback,
        dest,
        host_clipboard,
        notifier,
        supervisor,
        picker_command,
        accept_diag,
        candidates,
    );
}

fn open_picker_for_candidates(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut impl Notifier,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    accept_diag: &mut AcceptDiagnostics,
    candidates: Vec<Candidate>,
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
                let requested_mime = host_clipboard
                    .pending_paste()
                    .map(|paste| paste.mime_type.clone())
                    .unwrap_or_else(|| "text/plain".to_owned());
                if let Err(error) =
                    picker_handshake(socket, &request_id, &dest, &requested_mime, candidates)
                {
                    accept_diag.warn("picker", "handshake-failed", || {
                        format!("d2b-clipd: picker handshake failed: {error}")
                    });
                    let _ = supervisor.cancel_active(ReasonCode::PickerCrashed);
                    let _ = fallback.cancel_picker();
                    arm_native_fallback(fallback, dest, host_clipboard, notifier);
                } else {
                    log::debug!(
                        "d2b-clipd: picker opened for paste to {}",
                        bounded_label(dest.app_id.as_deref().unwrap_or("unknown"))
                    );
                }
            }
            Err(e) => {
                accept_diag.warn("picker", "launch-failed", || {
                    format!("d2b-clipd: picker launch failed ({e}); falling back to native paste")
                });
                let _ = fallback.cancel_picker();
                arm_native_fallback(fallback, dest, host_clipboard, notifier);
            }
        }
    } else {
        arm_native_fallback(fallback, dest, host_clipboard, notifier);
    }
}

fn arm_native_fallback(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut impl Notifier,
) {
    if let Some(paste) = host_clipboard.take_pending_paste() {
        paste.close_with_reason(ReasonCode::PickerNotConfigured);
    }
    if matches!(fallback.state(), FallbackState::Idle) {
        let _ = fallback.capture_target_before_picker(dest.clone());
    }
    let transition = fallback.arm_selected_entry(
        CURRENT_HOST_ENTRY_ID.to_owned(),
        Instant::now(),
        Duration::from_secs(30),
    );
    if matches!(transition, FallbackTransition::Armed) {
        let label = dest.app_id.as_deref().unwrap_or("host application");
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
                let snapshot = host_clipboard.apply_niri_cache_event(event.clone());
                if let NiriEvent::FocusChanged { .. } = &event {
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
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod control socket dir {}: {e}", parent.display()))?;
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
    use std::os::fd::AsRawFd;

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
        let mut control = ControlStream {
            stream: reader,
            read_buffer: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let err = read_control_command_from_stream(&mut control).expect_err("malformed");
        assert!(
            matches!(err, ControlReadError::Invalid(message) if message.contains("invalid control JSON"))
        );
    }

    #[test]
    fn nonblocking_control_response_writer_waits_for_writable_socket() {
        let (mut reader, writer) = UnixStream::pair().expect("pair");
        writer.set_nonblocking(true).expect("nonblocking writer");
        write_all_nonblocking_stream(&writer, b"{\"ok\":true}\n", Duration::from_secs(1))
            .expect("write response");
        drop(writer);
        let mut out = String::new();
        reader.read_to_string(&mut out).expect("read response");
        assert_eq!(out, "{\"ok\":true}\n");
    }

    #[test]
    fn nonblocking_control_response_writer_times_out_on_full_socket() {
        let (_reader, writer) = UnixStream::pair().expect("pair");
        writer.set_nonblocking(true).expect("nonblocking writer");
        let data = vec![b'x'; 16 * 1024 * 1024];
        let err = write_all_nonblocking_stream(&writer, &data, Duration::from_millis(5))
            .expect_err("full socket should time out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn idle_stream_reapers_drop_expired_partial_connections() {
        let now = Instant::now();
        let (_control_writer, control_reader) = UnixStream::pair().expect("control pair");
        let (_bridge_writer, bridge_reader) = UnixStream::pair().expect("bridge pair");
        let mut control_streams = vec![ControlStream {
            stream: control_reader,
            read_buffer: b"{".to_vec(),
            frame_deadline: now - Duration::from_millis(1),
        }];
        let mut bridge_streams = vec![BridgeStream {
            vm_name: "work".to_owned(),
            stream: bridge_reader,
            read_buffer: b"{".to_vec(),
            received_fds: Vec::new(),
            frame_deadline: now - Duration::from_millis(1),
        }];
        assert_eq!(reap_idle_control_streams(&mut control_streams, now), 1);
        assert_eq!(reap_idle_bridge_streams(&mut bridge_streams, now), 1);
        assert!(control_streams.is_empty());
        assert!(bridge_streams.is_empty());
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
    fn bridge_listener_socket_is_connectable_by_peer_group() {
        let root = std::env::temp_dir().join(format!(
            "d2b-clipd-bridge-test-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        let peer = BridgePeerConfig {
            vm_name: "work".to_owned(),
            expected_uid: rustix::process::getuid().as_raw(),
        };
        let listeners = install_bridge_listeners(&root, &[peer]).expect("install bridge listener");
        assert_eq!(listeners.len(), 1);
        let path = bridge_socket_path(&root, rustix::process::getuid().as_raw(), "work")
            .expect("bridge socket path");
        let mode = std::fs::symlink_metadata(&path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o666);
        drop(listeners);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fd_materialization_times_out_on_hanging_stream() {
        let (read_fd, _write_fd) = rustix::pipe::pipe().expect("pipe");
        let err = read_fd_to_vec(read_fd, 1024, Duration::from_millis(5)).expect_err("timeout");
        assert_eq!(err, ReasonCode::SourceMaterializeTimeout);
    }

    #[test]
    fn write_all_nonblocking_fd_writes_socketpair_bytes() {
        let (write_sock, mut read_sock) = UnixStream::pair().expect("pair");
        let fd: OwnedFd = write_sock.into();
        write_all_nonblocking_fd(&fd, b"hello", Instant::now() + Duration::from_secs(1))
            .expect("write");
        drop(fd);
        let mut out = Vec::new();
        read_sock.read_to_end(&mut out).expect("read");
        assert_eq!(out, b"hello");
    }

    #[test]
    fn write_all_nonblocking_fd_times_out_when_pipe_is_full() {
        use std::os::fd::AsFd;

        let (read_fd, write_fd) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
        rustix::io::ioctl_fionbio(write_fd.as_fd(), true).expect("nonblocking");
        let fill = vec![b'x'; 4096];
        loop {
            match rustix::io::write(&write_fd, &fill) {
                Ok(_) => {}
                Err(rustix::io::Errno::AGAIN) => break,
                Err(error) => panic!("unexpected pipe fill error: {error}"),
            }
        }
        let err = write_all_nonblocking_fd(
            &write_fd,
            b"blocked",
            Instant::now() + Duration::from_millis(5),
        )
        .expect_err("full pipe should time out");
        assert_eq!(err, ReasonCode::FdWriteTimeout);
        drop(read_fd);
        drop(write_fd);
    }

    #[test]
    fn spawn_materialize_to_pending_paste_writes_and_releases_pending_fd() {
        let mut host_clipboard = HostClipboard::new(
            HostClipboardAttributor::new(NiriQueryProvider::new(None)),
            Duration::from_secs(30),
        );
        let (paste_write, mut paste_read) = UnixStream::pair().expect("paste pair");
        host_clipboard
            .accept_paste_fd_for_destination(
                paste_write.into(),
                "text/plain".to_owned(),
                FocusedWindowSnapshot::default(),
            )
            .expect("accept paste");
        let (read_fd, write_fd) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
        rustix::io::write(&write_fd, b"hello").expect("write source");
        drop(write_fd);

        spawn_materialize_to_pending_paste(&mut host_clipboard, read_fd).expect("spawn");
        assert!(host_clipboard.pending_paste().is_none());

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut out = Vec::new();
        loop {
            let mut buf = [0_u8; 16];
            match paste_read.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "timed out waiting for paste bytes"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("read paste failed: {error}"),
            }
        }
        assert_eq!(out, b"hello");
    }

    #[test]
    fn bridge_frame_carries_exact_vm_metadata_and_fd() {
        let (sender, receiver) = UnixStream::pair().expect("bridge pair");
        let (read_fd, write_fd) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
        let frame = br#"{"type":"vm_paste_request","vm_name":"work","mime_type":"text/plain","source_id":7,"source_attribution":"exact_client"}
"#;
        let iov = [std::io::IoSlice::new(frame)];
        let raw_fd = write_fd.as_raw_fd();
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&[raw_fd])];
        nix::sys::socket::sendmsg::<()>(
            sender.as_raw_fd(),
            &iov,
            &cmsg,
            nix::sys::socket::MsgFlags::MSG_NOSIGNAL,
            None,
        )
        .expect("sendmsg");
        drop(write_fd);

        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        match recv_bridge_frame(&mut stream).expect("bridge frame") {
            BridgeRequest::Paste(request) => {
                assert_eq!(request.vm_name, "work");
                assert_eq!(request.mime_type, "text/plain");
                assert_eq!(request.source_id, 7);
                drop(request.fd);
            }
            other => panic!("expected paste request, got {other:?}"),
        }
        drop(read_fd);
    }

    #[test]
    fn bridge_frame_rejects_non_exact_attribution() {
        let (sender, receiver) = UnixStream::pair().expect("bridge pair");
        let (read_fd, write_fd) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
        let frame = br#"{"type":"vm_paste_request","vm_name":"work","mime_type":"text/plain","source_id":7,"source_attribution":"focused_window_guess"}
"#;
        let iov = [std::io::IoSlice::new(frame)];
        let raw_fd = write_fd.as_raw_fd();
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&[raw_fd])];
        nix::sys::socket::sendmsg::<()>(
            sender.as_raw_fd(),
            &iov,
            &cmsg,
            nix::sys::socket::MsgFlags::MSG_NOSIGNAL,
            None,
        )
        .expect("sendmsg");
        drop(write_fd);

        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let err = recv_bridge_frame(&mut stream).expect_err("non-exact attribution");
        assert!(
            err.message().contains("unknown variant")
                || err.message().contains("exact attribution")
        );
        drop(read_fd);
    }

    #[test]
    fn bridge_frame_rejects_more_than_one_fd() {
        let (sender, receiver) = UnixStream::pair().expect("bridge pair");
        let (read_a, write_a) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe a");
        let (read_b, write_b) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe b");
        let frame = br#"{"type":"vm_paste_request","vm_name":"work","mime_type":"text/plain","source_id":7,"source_attribution":"exact_client"}
"#;
        let iov = [std::io::IoSlice::new(frame)];
        let fds = [write_a.as_raw_fd(), write_b.as_raw_fd()];
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&fds)];
        nix::sys::socket::sendmsg::<()>(
            sender.as_raw_fd(),
            &iov,
            &cmsg,
            nix::sys::socket::MsgFlags::MSG_NOSIGNAL,
            None,
        )
        .expect("sendmsg");
        drop(write_a);
        drop(write_b);
        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let err = recv_bridge_frame(&mut stream).expect_err("too many fds rejected");
        assert!(err.message().contains("too many transfer fds"));
        drop(read_a);
        drop(read_b);
    }

    #[test]
    fn bridge_frame_rejects_overlong_fragment_without_newline() {
        let (mut sender, receiver) = UnixStream::pair().expect("bridge pair");
        sender
            .write_all(&vec![b'a'; BRIDGE_MAX_FRAME_BYTES + 1])
            .expect("write");
        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let err = recv_bridge_frame(&mut stream).expect_err("overlong bridge frame");
        assert!(err.message().contains("bridge frame too large"));
    }

    #[test]
    fn bridge_frame_allows_fd_for_partial_followup_frame() {
        let (sender, receiver) = UnixStream::pair().expect("bridge pair");
        let (read_a, write_a) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe a");
        let (read_b, write_b) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe b");
        let first = br#"{"type":"vm_paste_request","vm_name":"work","mime_type":"text/plain","source_id":7,"source_attribution":"exact_client"}
"#;
        let second_prefix = br#"{"#;
        let iov = [
            std::io::IoSlice::new(first),
            std::io::IoSlice::new(second_prefix),
        ];
        let fds = [write_a.as_raw_fd(), write_b.as_raw_fd()];
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&fds)];
        nix::sys::socket::sendmsg::<()>(
            sender.as_raw_fd(),
            &iov,
            &cmsg,
            nix::sys::socket::MsgFlags::MSG_NOSIGNAL,
            None,
        )
        .expect("sendmsg");
        drop(write_a);
        drop(write_b);
        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: Vec::new(),
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let request = match recv_bridge_frame(&mut stream).expect("first frame") {
            BridgeRequest::Paste(request) => request,
            other => panic!("expected paste request, got {other:?}"),
        };
        assert_eq!(request.source_id, 7);
        assert_eq!(stream.received_fds.len(), 1);
        assert_eq!(stream.read_buffer, b"{");
        drop(request.fd);
        drop(read_a);
        drop(read_b);
    }

    #[test]
    fn bridge_fd_queue_rejects_more_than_sixty_four_queued_fds() {
        let (_sender, receiver) = UnixStream::pair().expect("bridge pair");
        let mut reads = Vec::new();
        let mut received_fds = Vec::new();
        for _ in 0..65 {
            let (read, write) =
                rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
            reads.push(read);
            received_fds.push(write);
        }
        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds,
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let err = validate_bridge_fd_queue(&mut stream, rustix::net::RecvFlags::empty())
            .expect_err("queued fd cap rejected");
        assert!(err.message().contains("too many queued transfer fds"));
        assert!(stream.received_fds.is_empty());
        drop(reads);
    }

    #[test]
    fn bridge_fd_queue_rejects_ctrunc_and_clears_fds() {
        let (_sender, receiver) = UnixStream::pair().expect("bridge pair");
        let (read_fd, write_fd) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
        let mut stream = BridgeStream {
            vm_name: "work".to_owned(),
            stream: receiver,
            read_buffer: Vec::new(),
            received_fds: vec![write_fd],
            frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
        };
        let flags = rustix::net::RecvFlags::from_bits_retain(nix::libc::MSG_CTRUNC as u32);
        let err =
            validate_bridge_fd_queue(&mut stream, flags).expect_err("ctrunc must be rejected");
        assert!(err.message().contains("control message truncated"));
        assert!(stream.received_fds.is_empty());
        drop(read_fd);
    }

    #[test]
    fn recoverable_accept_errors_include_fd_exhaustion() {
        for errno in [
            nix::libc::ECONNABORTED,
            nix::libc::EINTR,
            nix::libc::EMFILE,
            nix::libc::ENFILE,
            nix::libc::ENOBUFS,
            nix::libc::ENOMEM,
        ] {
            let err = std::io::Error::from_raw_os_error(errno);
            assert!(is_recoverable_accept_error(&err));
        }
        let fatal = std::io::Error::from_raw_os_error(nix::libc::EACCES);
        assert!(!is_recoverable_accept_error(&fatal));
    }

    #[test]
    fn accept_backoff_masks_listener_poll_interest() {
        let now = Instant::now();
        assert!(accept_backoff_active(
            Some(now + Duration::from_millis(50)),
            now
        ));
        assert!(!accept_backoff_active(
            Some(now - Duration::from_millis(1)),
            now
        ));
        assert!(accept_listener_poll_flags(true).is_empty());
        assert_eq!(
            accept_listener_poll_flags(false),
            PollFlags::IN | PollFlags::ERR | PollFlags::HUP
        );
    }

    #[test]
    fn accept_diagnostics_rate_limit_and_flush_suppressed() {
        let mut diag = AcceptDiagnostics::default();
        diag.warn("control", "recoverable", || "first".to_owned());
        diag.warn("control", "recoverable", || "second".to_owned());
        let key = "control:recoverable".to_owned();
        assert_eq!(diag.suppressed.get(&key), Some(&1));
        diag.last_warn
            .insert(key.clone(), Instant::now() - ACCEPT_WARN_INTERVAL);
        diag.warn("control", "recoverable", || "third".to_owned());
        assert_eq!(diag.suppressed.get(&key), None);
    }

    #[test]
    fn bridge_peer_validation_checks_uid() {
        let (left, _right) = UnixStream::pair().expect("pair");
        let current = rustix::process::getuid().as_raw();
        validate_bridge_peer(&left, current).expect("current uid accepted");
        let wrong = if current == u32::MAX {
            current - 1
        } else {
            current + 1
        };
        let err = validate_bridge_peer(&left, wrong).expect_err("wrong uid rejected");
        assert!(err.contains("uid mismatch"));
    }

    #[test]
    fn background_probe_closes_pending_fd_without_clearing_arm() {
        let mut host_clipboard = HostClipboard::new(
            HostClipboardAttributor::new(NiriQueryProvider::new(None)),
            Duration::from_secs(30),
        );
        let (write_sock, mut read_sock) = UnixStream::pair().expect("pair");
        host_clipboard
            .accept_paste_fd_for_destination(
                write_sock.into(),
                "text/plain".to_owned(),
                FocusedWindowSnapshot {
                    id: Some(2),
                    app_id: Some("background".to_owned()),
                    title: None,
                    workspace_id: None,
                    output_label: None,
                },
            )
            .expect("accept");
        let target = FocusedWindowSnapshot {
            id: Some(1),
            app_id: Some("target".to_owned()),
            title: None,
            workspace_id: None,
            output_label: None,
        };
        assert!(reject_background_probe_if_target_mismatch(
            &target,
            &mut host_clipboard
        ));
        assert!(host_clipboard.pending_paste().is_none());
        let mut byte = [0_u8; 1];
        assert_eq!(read_sock.read(&mut byte).expect("eof"), 0);
    }
}
