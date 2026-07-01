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
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
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
    DesktopNotifier, Notifier, emit_fallback_ready, emit_user_visible_failure,
    sanitize_notification_text,
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
const MAX_HELPER_THREADS: usize = 16;
const ACCEPT_RESOURCE_BACKOFF: Duration = Duration::from_millis(50);
const ACCEPT_WARN_INTERVAL: Duration = Duration::from_secs(60);
const STREAM_FRAME_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const ASYNC_MATERIALIZE_POLL_INTERVAL: Duration = Duration::from_millis(50);

static HELPER_THREADS: AtomicUsize = AtomicUsize::new(0);

struct HelperThreadPermit;

impl Drop for HelperThreadPermit {
    fn drop(&mut self) {
        HELPER_THREADS.fetch_sub(1, Ordering::Release);
    }
}

fn try_acquire_helper_thread() -> Result<HelperThreadPermit, ReasonCode> {
    let mut current = HELPER_THREADS.load(Ordering::Acquire);
    loop {
        if current >= MAX_HELPER_THREADS {
            return Err(ReasonCode::FdCapExceeded);
        }
        match HELPER_THREADS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(HelperThreadPermit),
            Err(next) => current = next,
        }
    }
}

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

    let niri_socket: Option<PathBuf> = args
        .niri_socket
        .clone()
        .or_else(|| std::env::var("NIRI_SOCKET").ok().map(PathBuf::from));
    let (niri_tx, niri_rx) = mpsc::channel::<NiriMessage>();

    // ── Host clipboard state ─────────────────────────────────────────────────
    let niri_query = NiriQueryProvider::new(niri_socket);
    let attributor = HostClipboardAttributor::new(niri_query);
    let mut host_clipboard: HostClipboard<NiriQueryProvider> = HostClipboard::new(attributor);

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
    let (history_tx, history_rx) = mpsc::channel::<ClipboardHistoryEntry>();
    let (bridge_copy_tx, bridge_copy_rx) = mpsc::channel::<BridgeCopyReady>();

    // ── Control socket ───────────────────────────────────────────────────────
    let control_socket = control_socket_path()?;
    install_control_socket_parent(&control_socket)?;
    let listener =
        UnixListener::bind(&control_socket).map_err(|e| format!("bind control socket: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;
    let bridge_listeners = install_bridge_listeners(&args.bridge_root, &bridge_peers)?;

    // ── Niri IPC event stream thread ─────────────────────────────────────────
    let niri_socket = args
        .niri_socket
        .clone()
        .or_else(|| std::env::var("NIRI_SOCKET").ok().map(PathBuf::from));
    if let Some(ref socket) = niri_socket {
        spawn_niri_event_thread(socket.clone(), niri_tx);
    } else {
        log::warn!("d2b-clipd: NIRI_SOCKET not set; focused-window attribution unavailable");
    }

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
        published_selection: None,
        current_host_entry: None,
        history: ClipboardHistory::default(),
        history_tx,
        history_rx,
        bridge_copy_tx,
        bridge_copy_rx,
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
    published_selection: Option<PublishedSelectionState>,
    current_host_entry: Option<ClipboardHistoryEntry>,
    history: ClipboardHistory,
    history_tx: mpsc::Sender<ClipboardHistoryEntry>,
    history_rx: mpsc::Receiver<ClipboardHistoryEntry>,
    bridge_copy_tx: mpsc::Sender<BridgeCopyReady>,
    bridge_copy_rx: mpsc::Receiver<BridgeCopyReady>,
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
            self.drain_async_materialization();
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
                    published_selection: &mut self.published_selection,
                    current_host_entry: &mut self.current_host_entry,
                    history: &self.history,
                    history_tx: &self.history_tx,
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
                    let mut context = ControlHandlerContext {
                        supervisor: self.supervisor,
                        picker_command: &self.picker_command,
                        host_clipboard: self.host_clipboard,
                        fallback: self.fallback,
                        notifier: self.notifier,
                        accept_diag: &mut self.accept_diag,
                        history: &self.history,
                    };
                    match handle_control_stream(&mut stream, &mut context) {
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
                        fallback: self.fallback,
                        supervisor: self.supervisor,
                        picker_command: &self.picker_command,
                        accept_diag: &mut self.accept_diag,
                        audit_queue: self.audit_queue,
                        metrics_queue: self.metrics_queue,
                        published_selection: &mut self.published_selection,
                        history: &mut self.history,
                        current_host_entry: self.current_host_entry.as_ref(),
                        bridge_copy_tx: &self.bridge_copy_tx,
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
                        Ok(PickerPoll::Message(message)) => {
                            let mut context = PickerMessageContext {
                                data_control: self.data_control,
                                bridge_selection: self.bridge_selection.as_ref(),
                                published_selection: &mut self.published_selection,
                                bridge_streams: &mut self.bridge_streams,
                                current_host_entry: self.current_host_entry.as_ref(),
                                history: &self.history,
                                notifier: self.notifier,
                                fallback: self.fallback,
                                supervisor: self.supervisor,
                            };
                            handle_picker_message(message, &mut context);
                        }
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
            if let Some(_reason) = self.supervisor.reap_expired(now) {
                let _ = self.fallback.cancel_picker();
            }
            self.supervisor.reap_terminated(now);
            if let FallbackTransition::Cleared(r) = self.fallback.on_timeout(now) {
                log::debug!("d2b-clipd: paste action state cleared: {r:?}");
            }
            self.reap_idle_streams(now);
            self.accept_diag.flush_suppressed();
            flush_audit_events(self.audit_queue);
            flush_metric_events(self.metrics_queue);
        }
    }

    fn poll_timeout_ms(&self) -> i32 {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
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
                .min(ASYNC_MATERIALIZE_POLL_INTERVAL.as_millis())
                .min(i32::MAX as u128) as i32
        })
        .unwrap_or(ASYNC_MATERIALIZE_POLL_INTERVAL.as_millis() as i32)
    }

    fn drain_async_materialization(&mut self) {
        while let Ok(entry) = self.history_rx.try_recv() {
            if entry.source_realm_kind == RealmKind::Host {
                let mut current = entry.clone();
                current.entry_id = CURRENT_HOST_ENTRY_ID.to_owned();
                self.current_host_entry = Some(current);
                match publish_data_control_selection(
                    self.data_control,
                    entry.data_by_mime.clone(),
                    PublishedSelectionMode::Discovery,
                ) {
                    Ok(selection) => {
                        self.published_selection = Some(selection);
                        if let Err(error) = self.data_control.flush() {
                            self.accept_diag
                                .warn("host", "claim-selection-flush-failed", || {
                                    format!("d2b-clipd: host selection claim flush failed: {error}")
                                });
                        }
                        notify_bridge_selection_refresh(&mut self.bridge_streams);
                        log::info!(
                            "d2b-clipd: claimed host selection as discovery source mimes={}",
                            entry.data_by_mime.len()
                        );
                    }
                    Err(reason) => {
                        self.accept_diag.warn("host", "claim-selection-failed", || {
                            format!(
                                "d2b-clipd: host selection claim failed: {}",
                                reason.as_str()
                            )
                        });
                    }
                }
            }
            self.history.push(entry);
        }
        while let Ok(ready) = self.bridge_copy_rx.try_recv() {
            let mut context = BridgeCopyReadyContext {
                data_control: self.data_control,
                accept_diag: &mut self.accept_diag,
                bridge_selection: &mut self.bridge_selection,
                current_host_entry: &mut self.current_host_entry,
                history: &mut self.history,
            };
            handle_bridge_copy_ready(ready, &mut context);
            notify_bridge_selection_refresh(&mut self.bridge_streams);
        }
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
    history_entry_id: String,
    timestamp_unix_ms: u64,
    suppress_selection_echo: bool,
    source: Option<DataControlSource>,
    data_by_mime: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct PublishedSelectionState {
    data_control_source_id: u64,
    _source: DataControlSource,
    data_by_mime: BTreeMap<String, Vec<u8>>,
    mode: PublishedSelectionMode,
    suppress_selection_echo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishedSelectionMode {
    Discovery,
    Selected,
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
    fn push(&mut self, entry: ClipboardHistoryEntry) {
        if entry.data_by_mime.is_empty() {
            return;
        }
        self.push_front_bounded(entry);
    }

    fn upsert(&mut self, entry: ClipboardHistoryEntry) {
        if entry.data_by_mime.is_empty() {
            return;
        }
        if entry.entry_id.is_empty() {
            self.push(entry);
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|existing| existing.entry_id == entry.entry_id)
        {
            self.entries.remove(index);
        }
        self.entries.push_front(entry);
        while self.entries.len() > HISTORY_MAX_ENTRIES {
            self.entries.pop_back();
        }
    }

    fn push_front_bounded(&mut self, mut entry: ClipboardHistoryEntry) {
        self.next_id = self.next_id.saturating_add(1);
        entry.entry_id = format!("history-{}", self.next_id);
        self.entries.push_front(entry);
        while self.entries.len() > HISTORY_MAX_ENTRIES {
            self.entries.pop_back();
        }
    }

    fn candidates(&self, requested_mime_type: &str) -> Vec<Candidate> {
        self.candidates_excluding(requested_mime_type, None)
    }

    fn candidates_excluding(
        &self,
        requested_mime_type: &str,
        excluded_entry_id: Option<&str>,
    ) -> Vec<Candidate> {
        self.entries
            .iter()
            .filter(|entry| excluded_entry_id != Some(entry.entry_id.as_str()))
            .filter_map(|entry| {
                let bytes = compatible_mime_payload(&entry.data_by_mime, requested_mime_type)?;
                Some(Candidate {
                    entry_id: entry.entry_id.clone(),
                    source_realm: entry.source_realm.clone(),
                    source_realm_kind: entry.source_realm_kind,
                    source_app: entry.source_app.clone(),
                    source_app_id: entry.source_app_id.clone(),
                    source_attribution: entry.source_attribution,
                    preview_text: std::str::from_utf8(&bytes)
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

    fn data_for(&self, entry_id: &str) -> Option<BTreeMap<String, Vec<u8>>> {
        self.entries
            .iter()
            .find(|entry| entry.entry_id == entry_id)
            .map(|entry| entry.data_by_mime.clone())
    }
}

fn compatible_mime_payload(
    data_by_mime: &BTreeMap<String, Vec<u8>>,
    requested_mime: &str,
) -> Option<Vec<u8>> {
    if let Some(bytes) = data_by_mime.get(requested_mime) {
        return Some(bytes.clone());
    }
    if !is_text_plain_mime(requested_mime) {
        return None;
    }
    for fallback in ["text/plain;charset=utf-8", "text/plain"] {
        if let Some(bytes) = data_by_mime.get(fallback) {
            return Some(bytes.clone());
        }
    }
    if let Some(bytes) = data_by_mime.get("text/html")
        && let Some(text) = html_to_plain_text(bytes)
    {
        return Some(text.into_bytes());
    }
    None
}

fn bridge_history_entry_id(vm_name: &str, source_id: u64) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(vm_name.len().saturating_mul(2));
    for byte in vm_name.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("bridge-{encoded}-{source_id}")
}

fn html_to_plain_text(bytes: &[u8]) -> Option<String> {
    let text = html2text::config::plain_no_decorate()
        .string_from_read(bytes, 120)
        .ok()?;
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn compatible_mime_name<'a>(
    available_mimes: &'a [String],
    requested_mime: &str,
) -> Option<&'a str> {
    if let Some(mime) = available_mimes
        .iter()
        .find(|mime| mime.as_str() == requested_mime)
    {
        return Some(mime.as_str());
    }
    if !is_text_plain_mime(requested_mime) {
        return None;
    }
    for fallback in ["text/plain;charset=utf-8", "text/plain"] {
        if let Some(mime) = available_mimes
            .iter()
            .find(|mime| mime.as_str() == fallback)
        {
            return Some(mime.as_str());
        }
    }
    None
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
        // umask is process-wide: keep bridge listener installation in the
        // single-threaded startup phase, before spawning background workers.
        let old_umask = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o111));
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("bind bridge socket {}: {e}", path.display()));
        nix::sys::stat::umask(old_umask);
        let listener = listener?;
        let bound_meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("stat bound bridge socket {}: {e}", path.display()))?;
        if !bound_meta.file_type().is_socket() {
            return Err(format!("refusing bound non-socket {}", path.display()));
        }
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
                let mut stream = BridgeStream {
                    vm_name: bridge.vm_name.clone(),
                    stream,
                    read_buffer: Vec::new(),
                    received_fds: Vec::new(),
                    frame_deadline: Instant::now() + STREAM_FRAME_IDLE_TIMEOUT,
                };
                if !notify_bridge_stream_selection_refresh(&mut stream) {
                    continue;
                }
                streams.push(stream);
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
    fallback: &'a mut FallbackArming,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &'a Option<PickerCommand>,
    accept_diag: &'a mut AcceptDiagnostics,
    audit_queue: &'a mut AuditQueue,
    metrics_queue: &'a mut MetricsQueue,
    published_selection: &'a mut Option<PublishedSelectionState>,
    history: &'a mut ClipboardHistory,
    current_host_entry: Option<&'a ClipboardHistoryEntry>,
    bridge_copy_tx: &'a mpsc::Sender<BridgeCopyReady>,
}

struct BridgeCopyReady {
    vm_name: String,
    mime_type: String,
    source_id: u64,
    result: Result<Vec<u8>, ReasonCode>,
}

struct BridgeCopyReadyContext<'a> {
    data_control: &'a mut DataControlClient,
    accept_diag: &'a mut AcceptDiagnostics,
    bridge_selection: &'a mut Option<BridgeSelectionState>,
    current_host_entry: &'a mut Option<ClipboardHistoryEntry>,
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
        "d2b-clipd: received VM bridge paste request vm={} source_id:{} mime={}",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeRefreshWriteOutcome {
    Keep,
    Drop,
}

fn bridge_refresh_write_outcome(
    result: &std::io::Result<usize>,
    frame_len: usize,
) -> BridgeRefreshWriteOutcome {
    match result {
        Ok(written) if *written == frame_len => BridgeRefreshWriteOutcome::Keep,
        Ok(_) => BridgeRefreshWriteOutcome::Drop,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            BridgeRefreshWriteOutcome::Drop
        }
        Err(_) => BridgeRefreshWriteOutcome::Drop,
    }
}

fn notify_bridge_selection_refresh(streams: &mut Vec<BridgeStream>) {
    streams.retain_mut(notify_bridge_stream_selection_refresh);
}

fn notify_bridge_stream_selection_refresh(stream: &mut BridgeStream) -> bool {
    let frame = br#"{"type":"refresh_selection"}"#;
    let mut bytes = Vec::with_capacity(frame.len() + 1);
    bytes.extend_from_slice(frame);
    bytes.push(b'\n');
    let result = stream.stream.write(&bytes);
    match &result {
        Ok(n) if *n == bytes.len() => {}
        Ok(_) => {
            log::debug!(
                "d2b-clipd: bridge refresh notify partial write for vm={}; closing stream",
                bounded_label(&stream.vm_name)
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            log::debug!(
                "d2b-clipd: bridge refresh notify backpressured for vm={}; closing stream",
                bounded_label(&stream.vm_name)
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            log::debug!(
                "d2b-clipd: bridge refresh notify interrupted for vm={}; closing stream",
                bounded_label(&stream.vm_name)
            );
        }
        Err(error) => {
            log::debug!(
                "d2b-clipd: bridge refresh notify failed for vm={}: {}",
                bounded_label(&stream.vm_name),
                error
            );
        }
    }
    bridge_refresh_write_outcome(&result, bytes.len()) == BridgeRefreshWriteOutcome::Keep
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
    let tx = context.bridge_copy_tx.clone();
    let permit = match try_acquire_helper_thread() {
        Ok(permit) => permit,
        Err(reason) => {
            let _ = tx.send(BridgeCopyReady {
                vm_name,
                mime_type,
                source_id,
                result: Err(reason),
            });
            drop(fd);
            return;
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("d2b-clipd-bridge-copy-read".to_owned())
        .spawn(move || {
            let _permit = permit;
            let result = read_fd_to_vec(fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT);
            let _ = tx.send(BridgeCopyReady {
                vm_name,
                mime_type,
                source_id,
                result,
            });
        })
    {
        log::error!("d2b-clipd: failed to spawn bridge copy reader: {error}");
    }
}

fn handle_bridge_copy_ready(ready: BridgeCopyReady, context: &mut BridgeCopyReadyContext<'_>) {
    let BridgeCopyReady {
        vm_name,
        mime_type,
        source_id,
        result,
    } = ready;
    let bytes = match result {
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
    log::debug!(
        "d2b-clipd: bridge copy received vm={} source_id:{} mime={}",
        bounded_label(&vm_name),
        source_id,
        bounded_mime(&mime_type)
    );
    *context.current_host_entry = None;

    let replace = context.bridge_selection.as_ref().is_none_or(|selection| {
        selection.vm_name != vm_name || selection.vm_source_id != source_id
    });
    if replace {
        *context.bridge_selection = Some(BridgeSelectionState {
            vm_name: vm_name.clone(),
            vm_source_id: source_id,
            data_control_source_id: 0,
            history_entry_id: bridge_history_entry_id(&vm_name, source_id),
            timestamp_unix_ms: unix_millis(),
            suppress_selection_echo: false,
            source: None,
            data_by_mime: BTreeMap::new(),
        });
    }

    let Some(selection) = context.bridge_selection.as_mut() else {
        return;
    };
    selection.data_by_mime.insert(mime_type, bytes);
    context.history.upsert(ClipboardHistoryEntry {
        entry_id: selection.history_entry_id.clone(),
        source_realm: selection.vm_name.clone(),
        source_realm_kind: RealmKind::Vm,
        source_app: Some(format!("{} VM", selection.vm_name)),
        source_app_id: Some(format!("d2b.{}", selection.vm_name)),
        source_attribution: AttributionQuality::ExactClient,
        data_by_mime: selection.data_by_mime.clone(),
        timestamp_unix_ms: selection.timestamp_unix_ms,
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
            selection.source = Some(source);
            selection.data_control_source_id = source_id;
            selection.suppress_selection_echo = true;
            if let Err(error) = context.data_control.flush() {
                context.accept_diag.warn("bridge", "copy-flush-failed", || {
                    format!(
                        "d2b-clipd: bridge copy flush failed for vm={}: {error}",
                        bounded_label(&selection.vm_name)
                    )
                });
                return;
            }
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
        notify_bridge_paste_failure(context.notifier, ReasonCode::MimeRejected, "host", &vm_name);
        if let Err(reason) = context.audit_queue.enqueue_fail_closed(AuditEvent {
            request_id,
            source_realm: "host".to_owned(),
            destination_realm: vm_name,
            mime_type,
            byte_count: 0,
            decision: AuditDecision::Deny,
            attribution: d2b_clipd::policy::AttributionQuality::ExactClient,
            reason: ReasonCode::MimeRejected,
            timestamp_unix_ms: unix_millis(),
        }) {
            context.metrics_queue.enqueue_droppable(MetricEvent {
                name: MetricName::AuditQueueOverflow,
                reason: Some(reason),
            });
            context
                .accept_diag
                .warn("bridge", "audit-queue-failed", || {
                    format!(
                        "d2b-clipd: bridge deny audit queue failed: {}",
                        reason.as_str()
                    )
                });
            notify_bridge_paste_failure(context.notifier, reason, "host", "bridge");
        }
        flush_audit_events(context.audit_queue);
        flush_metric_events(context.metrics_queue);
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
        notify_bridge_paste_failure(context.notifier, reason, "host", &vm_name);
        return;
    }

    flush_audit_events(context.audit_queue);
    flush_metric_events(context.metrics_queue);
    if let Some(selection) = context.published_selection.as_ref()
        && selection.mode == PublishedSelectionMode::Selected
        && let Some(bytes) = compatible_mime_payload(&selection.data_by_mime, &mime_type)
    {
        log::debug!(
            "d2b-clipd: bridge paste served from published selection vm={} mime={}",
            bounded_label(&vm_name),
            bounded_mime(&mime_type)
        );
        spawn_write_bytes_to_fd(fd, mime_type, bytes);
        return;
    }
    drop(fd);
    let candidates = picker_candidates(
        context.host_clipboard,
        context.current_host_entry,
        context.history,
        &mime_type,
    );
    log::debug!(
        "d2b-clipd: bridge paste request vm={} source_id:{} mime={} dest_app={} dest_output={} candidates={} action=open-picker-and-replay",
        bounded_label(&vm_name),
        source_id,
        bounded_mime(&mime_type),
        bounded_label(dest.app_id.as_deref().unwrap_or("unknown")),
        bounded_label(dest.output_label.as_deref().unwrap_or("unknown")),
        summarize_candidates(&candidates)
    );
    let mut picker_context = PickerOpenContext {
        fallback: &mut *context.fallback,
        host_clipboard: &mut *context.host_clipboard,
        notifier: &mut *context.notifier,
        supervisor: &mut *context.supervisor,
        picker_command: context.picker_command,
        accept_diag: &mut *context.accept_diag,
    };
    open_picker_for_candidates(&mut picker_context, dest, &mime_type, candidates);
}

fn notify_bridge_paste_failure<N: Notifier>(
    notifier: &mut N,
    reason: ReasonCode,
    source_realm: &str,
    destination_realm: &str,
) {
    d2b_clipd::notifications::emit_user_visible_failure(
        notifier,
        reason,
        source_realm,
        destination_realm,
    );
}

fn flush_audit_events(audit_queue: &mut AuditQueue) {
    for event in audit_queue.drain_all() {
        match serde_json::to_string(&event) {
            Ok(json) => log::info!("d2b-clipd: audit_event {json}"),
            Err(error) => log::warn!("d2b-clipd: audit event encode failed: {error}"),
        }
    }
}

fn flush_metric_events(metrics_queue: &mut MetricsQueue) {
    let dropped = metrics_queue.take_dropped_count();
    if dropped > 0 {
        let event = MetricEvent {
            name: MetricName::DroppedDiagnostic,
            reason: None,
        };
        match serde_json::to_string(&event) {
            Ok(json) => log::warn!("d2b-clipd: metric_event {json} dropped_count:{dropped}"),
            Err(error) => log::warn!("d2b-clipd: metric event encode failed: {error}"),
        }
    }
    for event in metrics_queue.drain_all() {
        match serde_json::to_string(&event) {
            Ok(json) => log::debug!("d2b-clipd: metric_event {json}"),
            Err(error) => log::warn!("d2b-clipd: metric event encode failed: {error}"),
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
    published_selection: &'a mut Option<PublishedSelectionState>,
    current_host_entry: &'a mut Option<ClipboardHistoryEntry>,
    history: &'a ClipboardHistory,
    history_tx: &'a mpsc::Sender<ClipboardHistoryEntry>,
}

fn handle_wayland_event(event: HostClipboardEvent, context: &mut WaylandEventContext<'_>) {
    match event {
        HostClipboardEvent::SelectionChanged {
            offer,
            allowed_mimes,
            has_secret,
        } => {
            let focused_window = context.host_clipboard.focused_window_snapshot();
            if focused_window_matches_bridge_source(
                focused_window.as_ref(),
                context.bridge_selection.as_ref(),
            ) {
                context.host_clipboard.on_host_selection_cleared();
                log::debug!("d2b-clipd: ignored source-VM selection echo");
                return;
            }
            if context
                .published_selection
                .as_ref()
                .is_some_and(|selection| selection.suppress_selection_echo)
            {
                context.host_clipboard.on_host_selection_cleared();
                return;
            }
            if let Some(selection) = context.bridge_selection.as_mut()
                && selection.suppress_selection_echo
            {
                selection.suppress_selection_echo = false;
                context.host_clipboard.on_host_selection_cleared();
                return;
            }
            if context.bridge_selection.is_some() {
                *context.bridge_selection = None;
            }
            if context.published_selection.is_some() {
                *context.published_selection = None;
            }
            *context.current_host_entry = None;
            if let Some(offer_ref) = offer.as_ref() {
                let entry = ClipboardHistoryEntry {
                    entry_id: String::new(),
                    source_realm: "Host".to_owned(),
                    source_realm_kind: RealmKind::Host,
                    source_app: focused_window
                        .as_ref()
                        .and_then(|window| window.title.clone())
                        .or_else(|| Some("Host clipboard".to_owned())),
                    source_app_id: focused_window
                        .as_ref()
                        .and_then(|window| window.app_id.clone()),
                    source_attribution: AttributionQuality::FocusedWindowGuess,
                    data_by_mime: BTreeMap::new(),
                    timestamp_unix_ms: unix_millis(),
                };
                materialize_offer_mimes_async(
                    context.data_control,
                    offer_ref,
                    &allowed_mimes,
                    context.history_tx.clone(),
                    entry,
                );
            }
            log::info!(
                "d2b-clipd: host selection changed mimes={} attribution_app={} attribution_output={}",
                allowed_mimes.len(),
                bounded_label(
                    focused_window
                        .as_ref()
                        .and_then(|window| window.app_id.as_deref())
                        .unwrap_or("unknown")
                ),
                bounded_label(
                    focused_window
                        .as_ref()
                        .and_then(|window| window.output_label.as_deref())
                        .unwrap_or("unknown")
                )
            );
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
            if let Some(selection) = context.published_selection.as_ref()
                && selection.data_control_source_id == source_id
            {
                match selection.mode {
                    PublishedSelectionMode::Selected => {
                        if let Some(bytes) =
                            compatible_mime_payload(&selection.data_by_mime, &mime_type)
                        {
                            spawn_write_bytes_to_fd(fd, mime_type, bytes);
                        } else {
                            log::info!(
                                "d2b-clipd: published selection missing requested mime={}",
                                bounded_mime(&mime_type)
                            );
                            drop(fd);
                        }
                    }
                    PublishedSelectionMode::Discovery => {
                        drop(fd);
                        let dest = context
                            .host_clipboard
                            .refresh_focused_window_snapshot()
                            .unwrap_or_default();
                        let candidates = picker_candidates(
                            context.host_clipboard,
                            context.current_host_entry.as_ref(),
                            context.history,
                            &mime_type,
                        );
                        log::info!(
                            "d2b-clipd: discovery source paste request mime={} dest_app={} dest_output={} candidates={} action=open-picker-and-replay",
                            bounded_mime(&mime_type),
                            bounded_label(dest.app_id.as_deref().unwrap_or("unknown")),
                            bounded_label(dest.output_label.as_deref().unwrap_or("unknown")),
                            summarize_candidates(&candidates)
                        );
                        let mut picker_context = PickerOpenContext {
                            fallback: &mut *context.fallback,
                            host_clipboard: &mut *context.host_clipboard,
                            notifier: &mut *context.notifier,
                            supervisor: &mut *context.supervisor,
                            picker_command: context.picker_command,
                            accept_diag: &mut *context.accept_diag,
                        };
                        open_picker_for_candidates(
                            &mut picker_context,
                            dest,
                            &mime_type,
                            candidates,
                        );
                    }
                }
                return;
            }
            if let Some(selection) = context.bridge_selection.as_ref()
                && selection.data_control_source_id == source_id
            {
                if compatible_mime_payload(&selection.data_by_mime, &mime_type).is_none() {
                    context
                        .accept_diag
                        .warn("bridge", "selection-missing-mime", || {
                            format!(
                                "d2b-clipd: bridge selection missing requested mime={}",
                                bounded_mime(&mime_type)
                            )
                        });
                    drop(fd);
                    return;
                }
                let dest = context
                    .host_clipboard
                    .refresh_focused_window_snapshot()
                    .unwrap_or_default();
                drop(fd);
                if focused_app_matches_vm(dest.app_id.as_deref(), &selection.vm_name) {
                    log::debug!(
                        "d2b-clipd: ignored bridge source probe from source vm={} mime={}",
                        bounded_label(&selection.vm_name),
                        bounded_mime(&mime_type)
                    );
                    return;
                }
                let mut candidates = picker_bridge_candidates(selection, &mime_type);
                candidates.extend(
                    context
                        .history
                        .candidates_excluding(&mime_type, Some(&selection.history_entry_id)),
                );
                log::debug!(
                    "d2b-clipd: bridge selection paste request vm={} source_id:{} mime={} dest_app={} dest_output={} candidates={}",
                    bounded_label(&selection.vm_name),
                    selection.vm_source_id,
                    bounded_mime(&mime_type),
                    bounded_label(dest.app_id.as_deref().unwrap_or("unknown")),
                    bounded_label(dest.output_label.as_deref().unwrap_or("unknown")),
                    summarize_candidates(&candidates)
                );
                let mut picker_context = PickerOpenContext {
                    fallback: &mut *context.fallback,
                    host_clipboard: &mut *context.host_clipboard,
                    notifier: &mut *context.notifier,
                    supervisor: &mut *context.supervisor,
                    picker_command: context.picker_command,
                    accept_diag: &mut *context.accept_diag,
                };
                open_picker_for_candidates(&mut picker_context, dest, &mime_type, candidates);
                return;
            }
            drop(fd);
            let dest = context
                .host_clipboard
                .refresh_focused_window_snapshot()
                .unwrap_or_default();
            if focused_window_matches_bridge_source(Some(&dest), context.bridge_selection.as_ref())
            {
                log::debug!(
                    "d2b-clipd: ignored unknown d2b source probe from source VM mime={}",
                    bounded_mime(&mime_type)
                );
                return;
            }
            log::info!(
                "d2b-clipd: source send from unknown d2b source_id:{} mime={} opens picker",
                source_id,
                bounded_mime(&mime_type)
            );
            let mut picker_context = PickerOpenContext {
                fallback: &mut *context.fallback,
                host_clipboard: &mut *context.host_clipboard,
                notifier: &mut *context.notifier,
                supervisor: &mut *context.supervisor,
                picker_command: context.picker_command,
                accept_diag: &mut *context.accept_diag,
            };
            open_picker_or_arm_fallback(
                &mut picker_context,
                dest,
                context.current_host_entry.as_ref(),
                context.history,
            );
        }
        HostClipboardEvent::SourceCancelled { source_id } => {
            log::debug!("d2b-clipd: source {source_id} cancelled");
            if context
                .published_selection
                .as_ref()
                .is_some_and(|selection| selection.data_control_source_id == source_id)
            {
                *context.published_selection = None;
                context.host_clipboard.on_host_selection_cleared();
            }
            if context
                .bridge_selection
                .as_ref()
                .is_some_and(|selection| selection.data_control_source_id == source_id)
            {
                *context.bridge_selection = None;
                context.host_clipboard.on_host_selection_cleared();
            }
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

struct ControlHandlerContext<'a> {
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &'a Option<PickerCommand>,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    fallback: &'a mut FallbackArming,
    notifier: &'a mut DesktopNotifier,
    accept_diag: &'a mut AcceptDiagnostics,
    history: &'a ClipboardHistory,
}

fn handle_control_stream(
    control: &mut ControlStream,
    context: &mut ControlHandlerContext<'_>,
) -> ControlStreamStatus {
    match read_control_command_from_stream(control) {
        Ok(ControlCommand::Arm) => {
            let response = handle_arm(
                context.supervisor,
                context.picker_command,
                context.host_clipboard,
                context.fallback,
                context.notifier,
                context.accept_diag,
                context.history,
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

struct PickerMessageContext<'a> {
    data_control: &'a mut DataControlClient,
    bridge_selection: Option<&'a BridgeSelectionState>,
    published_selection: &'a mut Option<PublishedSelectionState>,
    bridge_streams: &'a mut Vec<BridgeStream>,
    current_host_entry: Option<&'a ClipboardHistoryEntry>,
    history: &'a ClipboardHistory,
    notifier: &'a mut DesktopNotifier,
    fallback: &'a mut FallbackArming,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
}

fn handle_picker_message(message: PickerToDaemonMessage, context: &mut PickerMessageContext<'_>) {
    match message {
        PickerToDaemonMessage::Select(select) => {
            log::info!(
                "d2b-clipd: picker selected entry for request {} entry={}",
                select.request_id,
                bounded_label(&select.entry_id)
            );
            match publish_selected_entry_to_host(
                context.data_control,
                context.bridge_selection,
                context.current_host_entry,
                context.history,
                &select.entry_id,
                context.published_selection,
            ) {
                Ok(()) => {
                    notify_bridge_selection_refresh(context.bridge_streams);
                    let _ = context.fallback.cancel_picker();
                    let _ = context.supervisor.cancel_active(ReasonCode::Allowed);
                    if let Err(error) = std::thread::Builder::new()
                        .name("d2b-clipd-paste-replay".to_owned())
                        .spawn(|| {
                            std::thread::sleep(Duration::from_millis(20));
                            if let Err(error) = d2b_clipd::virtual_keyboard::paste_ctrl_v() {
                                log::warn!("d2b-clipd: host instant paste failed: {error}");
                                let mut notifier = DesktopNotifier;
                                d2b_clipd::notifications::emit_user_visible_failure(
                                    &mut notifier,
                                    ReasonCode::VirtualKeyboardFailed,
                                    "clipboard",
                                    "host",
                                );
                            }
                        })
                    {
                        log::error!("d2b-clipd: failed to spawn paste replay worker: {error}");
                    }
                }
                Err(reason) => {
                    d2b_clipd::notifications::emit_user_visible_failure(
                        context.notifier,
                        reason,
                        "clipboard",
                        "host",
                    );
                    let _ = context.fallback.cancel_picker();
                    let _ = context.supervisor.cancel_active(reason);
                }
            }
        }
        PickerToDaemonMessage::Cancel(cancel) => {
            log::debug!("d2b-clipd: picker cancelled request {}", cancel.request_id);
            let _ = context.fallback.cancel_picker();
            let _ = context.supervisor.cancel_active(ReasonCode::PickerTimeout);
        }
        PickerToDaemonMessage::ClientHello(_) => {
            log::debug!("d2b-clipd: ignored duplicate picker client_hello");
        }
    }
}

fn publish_selected_entry_to_host(
    data_control: &mut DataControlClient,
    bridge_selection: Option<&BridgeSelectionState>,
    current_host_entry: Option<&ClipboardHistoryEntry>,
    history: &ClipboardHistory,
    entry_id: &str,
    published_selection: &mut Option<PublishedSelectionState>,
) -> Result<(), ReasonCode> {
    let data_by_mime =
        selected_entry_data_by_mime(bridge_selection, current_host_entry, history, entry_id)?;
    let selection = publish_data_control_selection(
        data_control,
        data_by_mime,
        PublishedSelectionMode::Selected,
    )?;
    let mimes_len = selection.data_by_mime.len();
    *published_selection = Some(selection);
    if data_control.flush().is_err() {
        *published_selection = None;
        return Err(ReasonCode::BridgeUnavailable);
    }
    log::info!(
        "d2b-clipd: published selected entry id={} mimes={} for instant paste",
        bounded_label(entry_id),
        mimes_len
    );
    Ok(())
}

fn publish_data_control_selection(
    data_control: &mut DataControlClient,
    data_by_mime: BTreeMap<String, Vec<u8>>,
    mode: PublishedSelectionMode,
) -> Result<PublishedSelectionState, ReasonCode> {
    if data_by_mime.is_empty() {
        return Err(ReasonCode::RequestExpired);
    }
    let mimes = preferred_mime_order(&data_by_mime);
    let (source, source_id) = data_control
        .create_source(&mimes)
        .map_err(|_| ReasonCode::BridgeUnavailable)?;
    data_control
        .set_selection(&source)
        .map_err(|_| ReasonCode::BridgeUnavailable)?;
    Ok(PublishedSelectionState {
        data_control_source_id: source_id,
        _source: source,
        data_by_mime,
        mode,
        suppress_selection_echo: true,
    })
}

fn selected_entry_data_by_mime(
    bridge_selection: Option<&BridgeSelectionState>,
    current_host_entry: Option<&ClipboardHistoryEntry>,
    history: &ClipboardHistory,
    entry_id: &str,
) -> Result<BTreeMap<String, Vec<u8>>, ReasonCode> {
    if entry_id == CURRENT_HOST_ENTRY_ID {
        return current_host_entry
            .map(|entry| entry.data_by_mime.clone())
            .filter(|data| !data.is_empty())
            .ok_or(ReasonCode::RequestExpired);
    }
    if entry_id == CURRENT_BRIDGE_ENTRY_ID {
        return bridge_selection
            .map(|selection| selection.data_by_mime.clone())
            .filter(|data| !data.is_empty())
            .ok_or(ReasonCode::RequestExpired);
    }
    history.data_for(entry_id).ok_or(ReasonCode::PolicyDenied)
}

fn preferred_mime_order(data_by_mime: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut out = Vec::new();
    for preferred in [
        "text/plain;charset=utf-8",
        "text/plain",
        "text/html",
        "image/png",
    ] {
        if data_by_mime.contains_key(preferred) {
            out.push(preferred.to_owned());
        }
    }
    for mime in data_by_mime.keys() {
        if !out.iter().any(|existing| existing == mime) {
            out.push(mime.clone());
        }
    }
    out
}

fn materialize_offer_mimes_async(
    data_control: &mut DataControlClient,
    offer: &d2b_clipd::wayland::DataControlOffer,
    allowed_mimes: &[String],
    tx: mpsc::Sender<ClipboardHistoryEntry>,
    mut entry: ClipboardHistoryEntry,
) {
    let mut reads = Vec::new();
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
        reads.push((mime.clone(), read_fd));
    }
    if reads.is_empty() {
        return;
    }
    let permit = match try_acquire_helper_thread() {
        Ok(permit) => permit,
        Err(reason) => {
            log::warn!("d2b-clipd: host copy reader denied: {}", reason.as_str());
            return;
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("d2b-clipd-host-copy-read".to_owned())
        .spawn(move || {
            let _permit = permit;
            for (mime, read_fd) in reads {
                if let Ok(bytes) =
                    read_fd_to_vec(read_fd, MATERIALIZE_MAX_BYTES, BOUNDED_READ_TIMEOUT)
                {
                    entry.data_by_mime.insert(mime, bytes);
                }
            }
            if !entry.data_by_mime.is_empty() {
                let _ = tx.send(entry);
            }
        })
    {
        log::error!("d2b-clipd: failed to spawn host copy reader: {error}");
    }
}

fn spawn_write_bytes_to_fd(fd: std::os::fd::OwnedFd, mime: String, bytes: Vec<u8>) {
    let permit = match try_acquire_helper_thread() {
        Ok(permit) => permit,
        Err(reason) => {
            log::info!(
                "d2b-clipd: published paste write denied mime={}: {}",
                bounded_mime(&mime),
                reason.as_str()
            );
            drop(fd);
            return;
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("d2b-clipd-published-write".to_owned())
        .spawn(move || {
            let _permit = permit;
            match write_all_nonblocking_fd(&fd, &bytes, Instant::now() + BOUNDED_READ_TIMEOUT) {
                Ok(()) => log::debug!(
                    "d2b-clipd: published paste write complete mime={}",
                    bounded_mime(&mime)
                ),
                Err(reason) => log::info!(
                    "d2b-clipd: published paste write failed mime={}: {}",
                    bounded_mime(&mime),
                    reason.as_str()
                ),
            }
            drop(fd);
        })
    {
        log::error!("d2b-clipd: failed to spawn published paste writer: {error}");
    }
}

fn is_text_plain_mime(mime: &str) -> bool {
    matches!(mime, "text/plain" | "text/plain;charset=utf-8")
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
/// We open the picker for the current focused window; the daemon owns the
/// eventual selection publication and paste replay.
fn handle_arm(
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    fallback: &mut FallbackArming,
    notifier: &mut impl Notifier,
    accept_diag: &mut AcceptDiagnostics,
    history: &ClipboardHistory,
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
            let requested_mime = "text/plain".to_owned();
            let candidates = picker_candidates(host_clipboard, None, history, &requested_mime);
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
                    emit_user_visible_failure(
                        notifier,
                        ReasonCode::PickerCrashed,
                        "clipboard",
                        dest.app_id.as_deref().unwrap_or("host"),
                    );
                    Err(ReasonCode::PickerCrashed.as_str().to_owned())
                }
            }
        }
        Err(e) => {
            accept_diag.warn("picker", "launch-failed", || {
                format!("d2b-clipd: picker launch failed: {e}")
            });
            let _ = fallback.cancel_picker();
            emit_user_visible_failure(
                notifier,
                ReasonCode::PickerNotConfigured,
                "clipboard",
                dest.app_id.as_deref().unwrap_or("host"),
            );
            Err(ReasonCode::PickerNotConfigured.as_str().to_owned())
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
    if let DaemonToPickerMessage::OpenRequest(request) = &request {
        log::info!(
            "d2b-clipd: picker open request id={} requested_mime={} dest_app={} dest_output={} candidates={}",
            bounded_label(&request.request_id),
            bounded_mime(&request.requested_mime_type),
            bounded_label(request.destination.app_id.as_deref().unwrap_or("unknown")),
            bounded_label(
                request
                    .placement_hints
                    .as_ref()
                    .and_then(|hints| hints.output.as_deref())
                    .unwrap_or("unknown")
            ),
            summarize_candidates(&request.candidates)
        );
    }
    let frame = encode_frame(&request, OpenRequestFrameCaps::default().max_frame_bytes())
        .map_err(|e| format!("encode open_request: {e}"))?;
    let writer = socket
        .try_clone()
        .map_err(|e| format!("clone for write: {e}"))?;
    write_all_nonblocking_stream(&writer, &frame, BOUNDED_READ_TIMEOUT)
        .map_err(|e| format!("write open_request: {e}"))?;

    Ok(picker_version)
}

fn picker_candidates(
    host_clipboard: &HostClipboard<NiriQueryProvider>,
    current_host_entry: Option<&ClipboardHistoryEntry>,
    history: &ClipboardHistory,
    requested_mime_type: &str,
) -> Vec<Candidate> {
    let mut candidates = history.candidates(requested_mime_type);
    if let Some(entry) = current_host_entry
        && let Some(bytes) = compatible_mime_payload(&entry.data_by_mime, requested_mime_type)
    {
        candidates.retain(|candidate| candidate.entry_id != CURRENT_HOST_ENTRY_ID);
        candidates.insert(
            0,
            Candidate {
                entry_id: CURRENT_HOST_ENTRY_ID.to_owned(),
                source_realm: entry.source_realm.clone(),
                source_realm_kind: entry.source_realm_kind,
                source_app: entry.source_app.clone(),
                source_app_id: entry.source_app_id.clone(),
                source_attribution: entry.source_attribution,
                preview_text: std::str::from_utf8(&bytes)
                    .ok()
                    .map(|text| sanitize_notification_text(text, 256)),
                content_type: requested_mime_type.to_owned(),
                timestamp_unix_ms: entry.timestamp_unix_ms,
                thumbnail_png_base64: None,
                byte_count: Some(bytes.len() as u64),
                confirmation_required: false,
            },
        );
        return candidates;
    }
    let Some(selection) = host_clipboard.current_selection() else {
        return candidates;
    };
    if selection.offer.is_none() || selection.allowed_mimes.is_empty() {
        return candidates;
    }
    if compatible_mime_name(&selection.allowed_mimes, requested_mime_type).is_none() {
        return candidates;
    }
    let window = selection.attribution.window.as_ref();
    insert_live_host_candidate(
        &mut candidates,
        window,
        selection.attribution.quality,
        requested_mime_type,
    );
    candidates
}

fn insert_live_host_candidate(
    candidates: &mut Vec<Candidate>,
    window: Option<&FocusedWindowSnapshot>,
    attribution: d2b_clipd::policy::AttributionQuality,
    requested_mime_type: &str,
) {
    candidates.retain(|candidate| candidate.entry_id != CURRENT_HOST_ENTRY_ID);
    candidates.insert(
        0,
        Candidate {
            entry_id: CURRENT_HOST_ENTRY_ID.to_owned(),
            source_realm: "Host".to_owned(),
            source_realm_kind: RealmKind::Host,
            source_app: window
                .and_then(|window| window.title.clone())
                .or_else(|| Some("Host clipboard".to_owned())),
            source_app_id: window.and_then(|window| window.app_id.clone()),
            source_attribution: protocol_attribution(attribution),
            preview_text: None,
            content_type: requested_mime_type.to_owned(),
            timestamp_unix_ms: unix_millis(),
            thumbnail_png_base64: None,
            byte_count: None,
            confirmation_required: false,
        },
    );
}

fn picker_bridge_candidates(
    selection: &BridgeSelectionState,
    requested_mime_type: &str,
) -> Vec<Candidate> {
    let Some(bytes) = compatible_mime_payload(&selection.data_by_mime, requested_mime_type) else {
        return Vec::new();
    };
    vec![Candidate {
        entry_id: CURRENT_BRIDGE_ENTRY_ID.to_owned(),
        source_realm: selection.vm_name.clone(),
        source_realm_kind: RealmKind::Vm,
        source_app: Some(format!("{} VM", selection.vm_name)),
        source_app_id: Some(format!("d2b.{}", selection.vm_name)),
        source_attribution: AttributionQuality::ExactClient,
        preview_text: std::str::from_utf8(&bytes)
            .ok()
            .map(|text| sanitize_notification_text(text, 256)),
        content_type: requested_mime_type.to_owned(),
        timestamp_unix_ms: unix_millis(),
        thumbnail_png_base64: None,
        byte_count: Some(bytes.len() as u64),
        confirmation_required: false,
    }]
}

fn summarize_candidates(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .take(8)
        .map(|candidate| {
            format!(
                "{}:{}:{}",
                bounded_label(&candidate.entry_id),
                bounded_label(&candidate.source_realm),
                candidate.source_realm_kind as u8
            )
        })
        .collect::<Vec<_>>()
        .join(",")
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

struct PickerOpenContext<'a> {
    fallback: &'a mut FallbackArming,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    notifier: &'a mut DesktopNotifier,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &'a Option<PickerCommand>,
    accept_diag: &'a mut AcceptDiagnostics,
}

fn open_picker_or_arm_fallback(
    context: &mut PickerOpenContext<'_>,
    dest: FocusedWindowSnapshot,
    current_host_entry: Option<&ClipboardHistoryEntry>,
    history: &ClipboardHistory,
) {
    let requested_mime = "text/plain".to_owned();
    let candidates = picker_candidates(
        context.host_clipboard,
        current_host_entry,
        history,
        &requested_mime,
    );
    open_picker_for_candidates(context, dest, &requested_mime, candidates);
}

fn open_picker_for_candidates(
    context: &mut PickerOpenContext<'_>,
    dest: FocusedWindowSnapshot,
    requested_mime: &str,
    candidates: Vec<Candidate>,
) {
    let can_open =
        context.picker_command.is_some() && matches!(context.supervisor.state(), PickerState::Idle);
    if can_open {
        let _ = context.fallback.capture_target_before_picker(dest.clone());
        let ambient: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        let request_id = format!("paste-{}", unix_millis());
        match context.supervisor.launch(
            request_id.clone(),
            context.picker_command.clone(),
            &ambient,
            Duration::from_secs(30),
        ) {
            Ok(socket) => {
                if let Err(error) =
                    picker_handshake(socket, &request_id, &dest, requested_mime, candidates)
                {
                    context.accept_diag.warn("picker", "handshake-failed", || {
                        format!("d2b-clipd: picker handshake failed: {error}")
                    });
                    let _ = context.supervisor.cancel_active(ReasonCode::PickerCrashed);
                    let _ = context.fallback.cancel_picker();
                    arm_native_fallback(
                        context.fallback,
                        dest,
                        context.host_clipboard,
                        context.notifier,
                    );
                } else {
                    log::debug!(
                        "d2b-clipd: picker opened for paste to {}",
                        bounded_label(dest.app_id.as_deref().unwrap_or("unknown"))
                    );
                }
            }
            Err(e) => {
                context.accept_diag.warn("picker", "launch-failed", || {
                    format!("d2b-clipd: picker launch failed ({e}); falling back to native paste")
                });
                let _ = context.fallback.cancel_picker();
                arm_native_fallback(
                    context.fallback,
                    dest,
                    context.host_clipboard,
                    context.notifier,
                );
            }
        }
    } else {
        arm_native_fallback(
            context.fallback,
            dest,
            context.host_clipboard,
            context.notifier,
        );
    }
}

fn arm_native_fallback(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    _host_clipboard: &mut HostClipboard<NiriQueryProvider>,
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
        let label = dest.app_id.as_deref().unwrap_or("host application");
        emit_fallback_ready(notifier, label);
        log::debug!("d2b-clipd: paste action armed for {}", bounded_label(label));
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
                None,
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

    fn query_workspaces(&mut self) -> Result<Vec<d2b_clipd::niri::NiriWorkspace>, NiriIpcError> {
        let Some(ref socket) = self.socket else {
            return Ok(Vec::new());
        };
        let mut client = NiriJsonClient::connect(
            socket,
            d2b_clipd::niri::DEFAULT_NIRI_MAX_LINE_BYTES,
            Some(Duration::from_secs(2)),
        )?;
        client.query_workspaces()
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

fn focused_app_matches_vm(app_id: Option<&str>, vm_name: &str) -> bool {
    let Some(app_id) = app_id else {
        return false;
    };
    app_id == format!("d2b.{vm_name}") || app_id.starts_with(&format!("d2b.{vm_name}."))
}

fn focused_window_matches_bridge_source(
    window: Option<&FocusedWindowSnapshot>,
    bridge_selection: Option<&BridgeSelectionState>,
) -> bool {
    let Some(selection) = bridge_selection else {
        return false;
    };
    focused_app_matches_vm(
        window.and_then(|window| window.app_id.as_deref()),
        &selection.vm_name,
    )
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
    use d2b_clipd::notifications::RecordingNotifier;
    use std::os::fd::AsRawFd;
    use std::sync::Mutex;

    static UMASK_TEST_LOCK: Mutex<()> = Mutex::new(());
    static HELPER_THREAD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn helper_thread_permits_are_bounded() {
        let _guard = HELPER_THREAD_TEST_LOCK.lock().expect("helper thread lock");
        HELPER_THREADS.store(0, Ordering::Release);
        let mut permits = Vec::new();
        for _ in 0..MAX_HELPER_THREADS {
            permits.push(try_acquire_helper_thread().expect("permit within cap"));
        }

        let err = match try_acquire_helper_thread() {
            Ok(_) => panic!("cap must reject extra helper"),
            Err(reason) => reason,
        };
        assert_eq!(err, ReasonCode::FdCapExceeded);

        drop(permits.pop());
        let permit = try_acquire_helper_thread().expect("released permit should be reusable");
        drop(permit);
        drop(permits);
        assert_eq!(HELPER_THREADS.load(Ordering::Acquire), 0);
    }

    #[test]
    fn focused_app_matching_identifies_source_vm_windows_only() {
        assert!(focused_app_matches_vm(
            Some("d2b.personal-dev.firefox"),
            "personal-dev"
        ));
        assert!(focused_app_matches_vm(
            Some("d2b.personal-dev"),
            "personal-dev"
        ));
        assert!(!focused_app_matches_vm(Some("firefox"), "personal-dev"));
        assert!(!focused_app_matches_vm(
            Some("d2b.work-ssd.firefox"),
            "personal-dev"
        ));
        assert!(!focused_app_matches_vm(
            Some("d2b.personal-devil.firefox"),
            "personal-dev"
        ));
    }

    #[test]
    fn focused_window_matching_suppresses_source_vm_echoes_only() {
        let selection = BridgeSelectionState {
            vm_name: "personal-dev".to_owned(),
            vm_source_id: 7,
            data_control_source_id: 11,
            history_entry_id: bridge_history_entry_id("personal-dev", 7),
            timestamp_unix_ms: 1,
            suppress_selection_echo: false,
            source: None,
            data_by_mime: BTreeMap::new(),
        };
        let source_vm_window = FocusedWindowSnapshot {
            app_id: Some("d2b.personal-dev.firefox".to_owned()),
            ..FocusedWindowSnapshot::default()
        };
        let host_window = FocusedWindowSnapshot {
            app_id: Some("firefox".to_owned()),
            ..FocusedWindowSnapshot::default()
        };

        assert!(focused_window_matches_bridge_source(
            Some(&source_vm_window),
            Some(&selection)
        ));
        assert!(!focused_window_matches_bridge_source(
            Some(&host_window),
            Some(&selection)
        ));
        assert!(!focused_window_matches_bridge_source(
            Some(&source_vm_window),
            None
        ));
    }

    #[test]
    fn handle_arm_reports_typed_error_when_picker_missing() {
        let mut supervisor = PickerSupervisor::new(CommandPickerSpawner);
        let picker_command = None;
        let mut host_clipboard = HostClipboard::new(d2b_clipd::niri::HostClipboardAttributor::new(
            NiriQueryProvider::new(None),
        ));
        let mut fallback = FallbackArming::default();
        let mut notifier = RecordingNotifier::default();
        let mut accept_diag = AcceptDiagnostics::default();
        let history = ClipboardHistory::default();

        let err = handle_arm(
            &mut supervisor,
            &picker_command,
            &mut host_clipboard,
            &mut fallback,
            &mut notifier,
            &mut accept_diag,
            &history,
        )
        .expect_err("missing picker must fail");

        assert_eq!(err, ReasonCode::PickerNotConfigured.as_str());
        assert!(matches!(fallback.state(), FallbackState::Idle));
        assert_eq!(notifier.notifications.len(), 1);
        assert!(notifier.notifications[0].body.contains("clipboard picker"));
    }

    #[test]
    fn compatible_mime_payload_converts_html_to_plain_text() {
        let mut data = BTreeMap::new();
        data.insert(
            "text/html".to_owned(),
            b"<p>Hello <strong>URL</strong> bar</p>".to_vec(),
        );

        let plain = compatible_mime_payload(&data, "text/plain").expect("plain fallback");
        assert_eq!(String::from_utf8(plain).expect("utf8"), "Hello URL bar");

        let charset =
            compatible_mime_payload(&data, "text/plain;charset=utf-8").expect("charset fallback");
        assert_eq!(String::from_utf8(charset).expect("utf8"), "Hello URL bar");
    }

    #[test]
    fn compatible_mime_payload_prefers_exact_plain_over_html() {
        let mut data = BTreeMap::new();
        data.insert("text/html".to_owned(), b"<p>HTML</p>".to_vec());
        data.insert("text/plain".to_owned(), b"plain".to_vec());

        let plain = compatible_mime_payload(&data, "text/plain").expect("plain");
        assert_eq!(plain, b"plain");
    }

    #[test]
    fn live_host_candidate_stays_ahead_of_older_host_history() {
        let mut candidates = vec![Candidate {
            entry_id: "history-1".to_owned(),
            source_realm: "Host".to_owned(),
            source_realm_kind: RealmKind::Host,
            source_app: Some("old copy".to_owned()),
            source_app_id: Some("old.app".to_owned()),
            source_attribution: AttributionQuality::FocusedWindowGuess,
            preview_text: Some("old".to_owned()),
            content_type: "text/plain".to_owned(),
            timestamp_unix_ms: 1,
            thumbnail_png_base64: None,
            byte_count: Some(3),
            confirmation_required: false,
        }];
        let window = FocusedWindowSnapshot {
            app_id: Some("firefox".to_owned()),
            title: Some("new copy target".to_owned()),
            ..FocusedWindowSnapshot::default()
        };

        insert_live_host_candidate(
            &mut candidates,
            Some(&window),
            d2b_clipd::policy::AttributionQuality::FocusedWindowGuess,
            "text/plain;charset=utf-8",
        );

        assert_eq!(candidates[0].entry_id, CURRENT_HOST_ENTRY_ID);
        assert_eq!(candidates[0].source_app.as_deref(), Some("new copy target"));
        assert_eq!(candidates[1].entry_id, "history-1");
    }

    #[test]
    fn current_vm_candidate_accepts_text_plain_aliases() {
        let mut data_by_mime = BTreeMap::new();
        data_by_mime.insert("text/plain".to_owned(), b"vm text".to_vec());
        let selection = BridgeSelectionState {
            vm_name: "personal-dev".to_owned(),
            vm_source_id: 7,
            data_control_source_id: 11,
            history_entry_id: bridge_history_entry_id("personal-dev", 7),
            timestamp_unix_ms: 1_700_000_000_000,
            suppress_selection_echo: false,
            source: None,
            data_by_mime,
        };

        let candidates = picker_bridge_candidates(&selection, "text/plain;charset=utf-8");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].entry_id, CURRENT_BRIDGE_ENTRY_ID);
        assert_eq!(candidates[0].preview_text.as_deref(), Some("vm text"));
        assert_eq!(candidates[0].byte_count, Some(7));
    }

    #[test]
    fn bridge_history_upsert_aggregates_mimes_without_duplicate_candidates() {
        let mut history = ClipboardHistory::default();
        let entry_id = bridge_history_entry_id("personal-dev", 7);
        let mut first = BTreeMap::new();
        first.insert("text/plain".to_owned(), b"plain".to_vec());
        history.upsert(ClipboardHistoryEntry {
            entry_id: entry_id.clone(),
            source_realm: "personal-dev".to_owned(),
            source_realm_kind: RealmKind::Vm,
            source_app: Some("personal-dev VM".to_owned()),
            source_app_id: Some("d2b.personal-dev".to_owned()),
            source_attribution: AttributionQuality::ExactClient,
            data_by_mime: first,
            timestamp_unix_ms: 1,
        });

        let mut second = BTreeMap::new();
        second.insert("text/plain".to_owned(), b"plain".to_vec());
        second.insert("text/html".to_owned(), b"<b>plain</b>".to_vec());
        history.upsert(ClipboardHistoryEntry {
            entry_id: entry_id.clone(),
            source_realm: "personal-dev".to_owned(),
            source_realm_kind: RealmKind::Vm,
            source_app: Some("personal-dev VM".to_owned()),
            source_app_id: Some("d2b.personal-dev".to_owned()),
            source_attribution: AttributionQuality::ExactClient,
            data_by_mime: second,
            timestamp_unix_ms: 1,
        });

        assert_eq!(history.entries.len(), 1);
        assert!(
            history
                .entries
                .front()
                .expect("entry")
                .data_by_mime
                .contains_key("text/html")
        );
        assert!(
            history
                .candidates_excluding("text/plain", Some(&entry_id))
                .is_empty()
        );
    }

    #[test]
    fn bridge_history_entry_id_is_injective_for_punctuated_vm_names() {
        assert_ne!(
            bridge_history_entry_id("work_vm", 7),
            bridge_history_entry_id("work-vm", 7)
        );
    }

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
        let _guard = UMASK_TEST_LOCK.lock().expect("umask lock");
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
    fn bridge_listener_temporary_umask_is_restored() {
        let _guard = UMASK_TEST_LOCK.lock().expect("umask lock");
        let root = std::env::temp_dir().join(format!(
            "d2b-clipd-bridge-umask-test-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        let peer = BridgePeerConfig {
            vm_name: "work".to_owned(),
            expected_uid: rustix::process::getuid().as_raw(),
        };
        let expected = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o027));
        nix::sys::stat::umask(expected);

        let old = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o027));
        let listeners = install_bridge_listeners(&root, &[peer]).expect("install bridge listener");
        let observed = nix::sys::stat::umask(old);

        assert_eq!(observed.bits() & 0o777, 0o027);
        drop(listeners);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bridge_refresh_partial_write_drops_stream() {
        assert_eq!(
            bridge_refresh_write_outcome(&Ok(3), br#"{"type":"refresh_selection"}"#.len() + 1),
            BridgeRefreshWriteOutcome::Drop
        );
        assert_eq!(
            bridge_refresh_write_outcome(
                &Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
                br#"{"type":"refresh_selection"}"#.len() + 1,
            ),
            BridgeRefreshWriteOutcome::Drop
        );
    }

    #[test]
    fn bridge_paste_audit_failure_emits_user_visible_notification() {
        let mut notifier = RecordingNotifier::default();

        notify_bridge_paste_failure(&mut notifier, ReasonCode::AuditFailure, "host", "work");

        assert_eq!(notifier.notifications.len(), 1);
        assert_eq!(
            notifier.notifications[0].summary,
            "d2b clipboard paste blocked"
        );
        assert!(notifier.notifications[0].body.contains("audit queue"));
    }

    #[test]
    fn audit_flush_drains_visible_events() {
        let mut queue = AuditQueue::new(AuditQueueConfig { per_realm_quota: 4 });
        queue
            .enqueue_fail_closed(AuditEvent {
                request_id: "req".to_owned(),
                source_realm: "host".to_owned(),
                destination_realm: "work".to_owned(),
                mime_type: "text/plain".to_owned(),
                byte_count: 0,
                decision: AuditDecision::Allow,
                attribution: d2b_clipd::policy::AttributionQuality::ExactClient,
                reason: ReasonCode::Allowed,
                timestamp_unix_ms: 1,
            })
            .expect("enqueue");

        flush_audit_events(&mut queue);

        assert_eq!(queue.len_for_realm("host"), 0);
    }

    #[test]
    fn metric_flush_emits_dropped_diagnostic_and_drains_queue() {
        let mut queue = MetricsQueue::new(1);
        queue.enqueue_droppable(MetricEvent {
            name: MetricName::PickerOpened,
            reason: None,
        });
        queue.enqueue_droppable(MetricEvent {
            name: MetricName::PickerTimeout,
            reason: Some(ReasonCode::PickerTimeout),
        });

        flush_metric_events(&mut queue);

        assert_eq!(queue.take_dropped_count(), 0);
        assert!(queue.is_empty());
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
}
