//! d2b-clipd: host-session clipboard authority daemon.
//!
//! Connects to the host Wayland compositor via the data-control protocol,
//! subscribes to Niri IPC events for focused-window attribution, supervises
//! the picker process, and drives the native-paste fallback state machine.
//!
//! No raw clipboard contents are ever logged.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use d2b_clipd::fallback::{FallbackArming, FallbackState, FallbackTransition};
use d2b_clipd::framing::{
    OpenRequestFrameCaps, PICKER_TO_DAEMON_MAX_FRAME_BYTES, decode_frame, encode_frame,
};
use d2b_clipd::host::HostClipboard;
use d2b_clipd::niri::{
    FocusedWindowSnapshot, HostClipboardAttributor, NiriEvent, NiriIpcError, NiriJsonClient,
    NiriRequest,
};
use d2b_clipd::notifications::{DesktopNotifier, Notifier, emit_fallback_ready};
use d2b_clipd::picker::{CommandPickerSpawner, PickerCommand, PickerState, PickerSupervisor};
use d2b_clipd::policy::ReasonCode;
use d2b_clipd::protocol::{
    AttributionQuality, ClientHello, DaemonToPickerMessage, DestinationMetadata, OpenRequest,
    PickerToDaemonMessage, RealmKind,
};
use d2b_clipd::wayland::{DataControlClient, HostClipboardEvent};

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

            // ── Wayland events ────────────────────────────────────────────────
            self.data_control.prepare_and_read().ok();
            let wl_events = self.data_control.dispatch_pending().unwrap_or_else(|e| {
                log::error!("d2b-clipd: wayland dispatch: {e}");
                vec![]
            });
            for event in wl_events {
                handle_wayland_event(
                    event,
                    self.host_clipboard,
                    self.notifier,
                    self.fallback,
                    self.supervisor,
                    &self.picker_command,
                );
            }

            // ── Control socket accepts ────────────────────────────────────────
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
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

            // ── Niri event channel (mpsc, drained each iteration) ─────────────
            drain_niri_channel(&self.niri_rx, self.host_clipboard, self.fallback);

            // ── Periodic timeout checks ───────────────────────────────────────
            let now = Instant::now();
            if let Some(expired) = self.host_clipboard.check_paste_timeout(now) {
                log::debug!("d2b-clipd: paste fd timed out (mime={})", expired.mime_type);
                expired.close_with_reason(ReasonCode::FdWriteTimeout);
            }
            if let Some(_reason) = self.supervisor.reap_expired(now) {
                let _ = self.fallback.cancel_picker();
            }
            if let FallbackTransition::Cleared(r) = self.fallback.on_timeout(now) {
                log::debug!("d2b-clipd: fallback armed state cleared: {r:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// ─── Wayland event handler ────────────────────────────────────────────────────

fn handle_wayland_event(
    event: HostClipboardEvent,
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
                        "d2b-clipd: paste fd held for mime={mime_type} dest={:?}",
                        dest.app_id
                    );
                    open_picker_or_arm_fallback(
                        fallback,
                        dest,
                        notifier,
                        supervisor,
                        picker_command,
                    );
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
            let picker_version = picker_handshake(socket, &request_id, &dest).unwrap_or_else(|e| {
                log::warn!("d2b-clipd: picker handshake failed: {e}");
                "unknown".to_owned()
            });
            log::debug!("d2b-clipd: picker opened (version={picker_version})");
            Ok("picker opened".to_owned())
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
) -> Result<String, String> {
    let mut reader = BufReader::new(
        socket
            .try_clone()
            .map_err(|e| format!("clone socket: {e}"))?,
    );
    let mut hello_buf = Vec::new();
    reader
        .read_until(b'\n', &mut hello_buf)
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
        candidates: Vec::new(),
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

// ─── Picker / fallback helpers ────────────────────────────────────────────────

fn open_picker_or_arm_fallback(
    fallback: &mut FallbackArming,
    dest: FocusedWindowSnapshot,
    notifier: &mut impl Notifier,
    supervisor: &mut PickerSupervisor<CommandPickerSpawner>,
    picker_command: &Option<PickerCommand>,
) {
    let can_open = picker_command.is_some() && matches!(supervisor.state(), PickerState::Idle);
    if can_open {
        let _ = fallback.capture_target_before_picker(dest.clone());
        let ambient: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        match supervisor.launch(
            format!("paste-{}", unix_millis()),
            picker_command.clone(),
            &ambient,
            Duration::from_secs(30),
        ) {
            Ok(_socket) => {
                log::debug!("d2b-clipd: picker opened for paste to {:?}", dest.app_id);
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
    let entry_id = format!("host-{}", unix_millis());
    let transition = fallback.arm_selected_entry(entry_id, Instant::now(), Duration::from_secs(30));
    if matches!(transition, FallbackTransition::Armed) {
        let label = dest
            .app_id
            .as_deref()
            .or(dest.title.as_deref())
            .unwrap_or("host application");
        emit_fallback_ready(notifier, label);
        log::debug!("d2b-clipd: fallback armed for {label}; user should press Ctrl+V");
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
    Ok(PathBuf::from(runtime).join("d2b/clipd.sock"))
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

fn read_control_command(stream: &UnixStream) -> Result<ControlCommand, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| format!("clone control stream: {e}"))?,
    );
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("read control command: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&line).map_err(|e| format!("invalid control JSON: {e}"))?;
    match value.get("type").and_then(|v| v.as_str()) {
        Some("arm") => Ok(ControlCommand::Arm),
        other => Err(format!("unknown control command: {other:?}")),
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
}
