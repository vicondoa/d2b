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
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use d2b_clipd::audit::{AuditDecision, AuditEvent, AuditQueue, AuditQueueConfig, bounded_mime};
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
use d2b_clipd::policy::{ReasonCode, is_mime_allowed};
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
        data_control: &mut data_control,
        niri_rx,
        host_clipboard: &mut host_clipboard,
        supervisor: &mut supervisor,
        picker_command,
        fallback: &mut fallback,
        notifier: &mut notifier,
        audit_queue: &mut audit_queue,
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
    data_control: &'a mut DataControlClient,
    niri_rx: mpsc::Receiver<NiriMessage>,
    host_clipboard: &'a mut HostClipboard<NiriQueryProvider>,
    supervisor: &'a mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: Option<PickerCommand>,
    fallback: &'a mut FallbackArming,
    notifier: &'a mut DesktopNotifier,
    audit_queue: &'a mut AuditQueue,
}

impl EventLoop<'_> {
    fn run(&mut self) -> Result<(), String> {
        loop {
            // Flush pending Wayland requests before polling.
            self.data_control.flush().ok();

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
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
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
                        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
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
                    .filter_map(|(index, fd)| fd.revents().contains(PollFlags::IN).then_some(index))
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
                    poll_fds[1].revents().contains(PollFlags::IN),
                    picker_ready,
                    control_stream_ready,
                    bridge_listener_ready,
                    bridge_stream_ready,
                )
            };

            drain_niri_channel(&self.niri_rx, self.host_clipboard, self.fallback);

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
                            self.control_streams.push(ControlStream {
                                stream,
                                read_buffer: Vec::new(),
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
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
                    ) {
                        ControlStreamStatus::Done => {}
                        ControlStreamStatus::Incomplete => self.control_streams.push(stream),
                    }
                }
            }

            for index in bridge_listener_ready {
                if let Some(bridge) = self.bridge_listeners.get(index) {
                    accept_bridge_streams(bridge, &mut self.bridge_streams);
                }
            }

            for index in bridge_stream_ready.into_iter().rev() {
                if index < self.bridge_streams.len() {
                    let mut stream = self.bridge_streams.swap_remove(index);
                    match handle_bridge_stream(
                        &mut stream,
                        self.host_clipboard,
                        self.notifier,
                        self.fallback,
                        self.supervisor,
                        &self.picker_command,
                        self.audit_queue,
                    ) {
                        BridgeStreamStatus::Done => {}
                        BridgeStreamStatus::Incomplete => self.bridge_streams.push(stream),
                    }
                }
            }

            // ── Picker responses ──────────────────────────────────────────────
            if picker_ready {
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
                        let _ = self.supervisor.cancel_active(ReasonCode::PickerCrashed);
                    }
                    Ok(PickerPoll::Incomplete) => {}
                    Err(error) => {
                        log::warn!("d2b-clipd: picker frame failed: {error}");
                        let _ = self.fallback.cancel_picker();
                        let _ = self.supervisor.cancel_active(ReasonCode::PickerCrashed);
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
            if let FallbackTransition::Cleared(r) = self.fallback.on_timeout(now) {
                log::debug!("d2b-clipd: fallback armed state cleared: {r:?}");
            }
        }
    }

    fn poll_timeout_ms(&self) -> i32 {
        let now = Instant::now();
        let mut next = self.host_clipboard.pending_paste_deadline();
        if let Some(deadline) = self.supervisor.deadline() {
            next = Some(next.map_or(deadline, |old| old.min(deadline)));
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
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
        std::fs::create_dir_all(parent)
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

fn accept_bridge_streams(bridge: &BridgeListener, streams: &mut Vec<BridgeStream>) {
    loop {
        match bridge.listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = stream.set_nonblocking(true) {
                    log::warn!("d2b-clipd: bridge stream nonblocking failed: {error}");
                    continue;
                }
                if let Err(error) = validate_bridge_peer(&stream, bridge.expected_uid) {
                    log::warn!(
                        "d2b-clipd: bridge peer rejected for vm={}: {error}",
                        bridge.vm_name
                    );
                    continue;
                }
                streams.push(BridgeStream {
                    vm_name: bridge.vm_name.clone(),
                    stream,
                    read_buffer: Vec::new(),
                    received_fds: Vec::new(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => {
                log::warn!(
                    "d2b-clipd: bridge accept failed for vm={}: {error}",
                    bridge.vm_name
                );
                break;
            }
        }
    }
}

enum BridgeStreamStatus {
    Done,
    Incomplete,
}

fn handle_bridge_stream(
    bridge: &mut BridgeStream,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut DesktopNotifier,
    fallback: &mut FallbackArming,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    audit_queue: &mut AuditQueue,
) -> BridgeStreamStatus {
    match recv_bridge_frame(bridge) {
        Ok(request) => handle_bridge_paste_request(
            request,
            host_clipboard,
            notifier,
            fallback,
            supervisor,
            picker_command,
            audit_queue,
        ),
        Err(BridgeReadError::Incomplete) => return BridgeStreamStatus::Incomplete,
        Err(error) => {
            log::warn!(
                "d2b-clipd: bridge frame failed for vm={}: {}",
                bridge.vm_name,
                error.message()
            );
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

fn recv_bridge_frame(stream: &mut BridgeStream) -> Result<BridgePasteRequest, BridgeReadError> {
    loop {
        let mut buf = [0_u8; 4096];
        let mut iov = [std::io::IoSliceMut::new(&mut buf)];
        let mut cmsg_space = [0_u8; rustix::cmsg_space!(ScmRights(1))];
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
            Err(error) => return Err(BridgeReadError::Invalid(error.to_string())),
        };
        if msg.flags.bits() & (nix::libc::MSG_CTRUNC as u32) != 0 {
            return Err(BridgeReadError::Invalid(
                "bridge control message truncated".to_owned(),
            ));
        }
        if msg.bytes == 0 {
            return Err(BridgeReadError::Invalid(
                "bridge stream closed before complete frame".to_owned(),
            ));
        }
        stream.read_buffer.extend_from_slice(&buf[..msg.bytes]);
        for cmsg in control.drain() {
            if let rustix::net::RecvAncillaryMessage::ScmRights(fds) = cmsg {
                stream.received_fds.extend(fds);
            }
        }
        if stream.read_buffer.len() > 4096 {
            return Err(BridgeReadError::Invalid(
                "bridge frame too large".to_owned(),
            ));
        }
        if stream.read_buffer.contains(&b'\n') {
            return parse_bridge_frame(stream);
        }
    }
}

fn parse_bridge_frame(stream: &mut BridgeStream) -> Result<BridgePasteRequest, BridgeReadError> {
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
        } => {
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
            log::debug!(
                "d2b-clipd: received VM bridge paste request vm={} source_id={} mime={}",
                bounded_label(&stream.vm_name),
                source_id,
                bounded_mime(&mime_type)
            );
            Ok(BridgePasteRequest {
                vm_name,
                mime_type,
                source_id,
                fd,
            })
        }
    }
}

fn handle_bridge_paste_request(
    request: BridgePasteRequest,
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
    notifier: &mut DesktopNotifier,
    fallback: &mut FallbackArming,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
    audit_queue: &mut AuditQueue,
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
        d2b_clipd::notifications::emit_user_visible_failure(
            notifier,
            ReasonCode::MimeRejected,
            "host",
            &vm_name,
        );
        let _ = audit_queue.enqueue_fail_closed(AuditEvent {
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
        return;
    }
    let dest = FocusedWindowSnapshot {
        id: None,
        app_id: Some(format!("d2b.{vm_name}")),
        title: Some(format!("{vm_name} VM")),
        workspace_id: None,
        output_label: None,
    };
    if let Err(reason) = audit_queue.enqueue_fail_closed(AuditEvent {
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
        log::warn!(
            "d2b-clipd: bridge audit queue failed for vm={}: {}",
            bounded_label(&vm_name),
            reason.as_str()
        );
        return;
    }
    match host_clipboard.accept_paste_fd_for_destination(fd, mime_type, dest.clone()) {
        Ok(dest) if picker_command.is_some() && matches!(supervisor.state(), PickerState::Idle) => {
            let _ = fallback.capture_target_before_picker(dest.clone());
            let ambient: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
            match supervisor.launch(
                request_id.clone(),
                picker_command.clone(),
                &ambient,
                Duration::from_secs(30),
            ) {
                Ok(socket) => {
                    let candidates = picker_candidates(host_clipboard);
                    if let Err(error) = picker_handshake(socket, &request_id, &dest, candidates) {
                        log::warn!("d2b-clipd: bridge picker handshake failed: {error}");
                        if let Some(paste) = host_clipboard.take_pending_paste() {
                            paste.close_with_reason(ReasonCode::PickerCrashed);
                        }
                        let _ = supervisor.cancel_active(ReasonCode::PickerCrashed);
                        let _ = fallback.cancel_picker();
                    }
                }
                Err(error) => {
                    log::warn!("d2b-clipd: bridge picker launch failed: {error}");
                    if let Some(paste) = host_clipboard.take_pending_paste() {
                        paste.close_with_reason(ReasonCode::PickerNotConfigured);
                    }
                    let _ = fallback.cancel_picker();
                }
            }
        }
        Ok(_) => {
            if let Some(paste) = host_clipboard.take_pending_paste() {
                paste.close_with_reason(ReasonCode::PickerNotConfigured);
            }
            d2b_clipd::notifications::emit_user_visible_failure(
                notifier,
                ReasonCode::PickerNotConfigured,
                "host",
                &vm_name,
            );
        }
        Err(reason) => {
            d2b_clipd::notifications::emit_user_visible_failure(notifier, reason, "host", &vm_name);
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

struct ControlStream {
    stream: UnixStream,
    read_buffer: Vec<u8>,
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
) -> ControlStreamStatus {
    match read_control_command_from_stream(control) {
        Ok(ControlCommand::Arm) => {
            let response = handle_arm(
                supervisor,
                picker_command,
                host_clipboard,
                fallback,
                notifier,
            );
            let body = match response {
                Ok(msg) => format!("{{\"ok\":true,\"message\":{}}}\n", json_string(&msg)),
                Err(err) => format!("{{\"ok\":false,\"error\":{}}}\n", json_string(&err)),
            };
            if let Err(error) = control.stream.write_all(body.as_bytes()) {
                log::warn!("d2b-clipd: write control response failed: {error}");
            }
            ControlStreamStatus::Done
        }
        Err(ControlReadError::Incomplete) => ControlStreamStatus::Incomplete,
        Err(ControlReadError::Invalid(error)) => {
            let body = format!("{{\"ok\":false,\"error\":{}}}\n", json_string(&error));
            if let Err(error) = control.stream.write_all(body.as_bytes()) {
                log::warn!("d2b-clipd: write control error response failed: {error}");
            }
            ControlStreamStatus::Done
        }
    }
}

#[derive(Debug)]
enum ControlReadError {
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
                return Err(ControlReadError::Invalid(
                    "peer closed before newline".to_owned(),
                ));
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
    let Some(paste) = host_clipboard.pending_paste() else {
        return false;
    };
    if !target.same_target(&paste.destination) {
        if let Some(paste) = host_clipboard.take_pending_paste() {
            paste.close_with_reason(ReasonCode::BackgroundProbe);
        }
        let _ = fallback.cancel_picker();
        return true;
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
    let (read_fd, write_fd) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        .map_err(|_| ReasonCode::FdClosed)?;
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
                    arm_native_fallback(fallback, dest.clone(), host_clipboard, notifier);
                    Err(error)
                }
            }
        }
        Err(e) => {
            log::warn!("d2b-clipd: picker launch failed: {e}; arming native fallback");
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
    host_clipboard: &mut HostClipboard<NiriQueryProvider>,
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
                    arm_native_fallback(fallback, dest, host_clipboard, notifier);
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
        };
        let err = read_control_command_from_stream(&mut control).expect_err("malformed");
        assert!(
            matches!(err, ControlReadError::Invalid(message) if message.contains("invalid control JSON"))
        );
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
        };
        let request = recv_bridge_frame(&mut stream).expect("bridge frame");
        assert_eq!(request.vm_name, "work");
        assert_eq!(request.mime_type, "text/plain");
        assert_eq!(request.source_id, 7);
        drop(request.fd);
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
        };
        let err = recv_bridge_frame(&mut stream).expect_err("non-exact attribution");
        assert!(
            err.message().contains("unknown variant")
                || err.message().contains("exact attribution")
        );
        drop(read_fd);
    }
}
