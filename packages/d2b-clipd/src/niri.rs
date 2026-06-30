use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::policy::AttributionQuality;

pub const DEFAULT_NIRI_MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum NiriIpcError {
    #[error("failed to connect to niri socket: {0}")]
    Connect(String),
    #[error("niri ipc I/O failed: {0}")]
    Io(String),
    #[error("niri ipc frame exceeds {max} bytes")]
    FrameTooLong { max: usize },
    #[error("niri ipc frame ended before newline")]
    Incomplete,
    #[error("niri ipc frame is not utf-8")]
    InvalidUtf8,
    #[error("niri ipc json error: {0}")]
    Json(String),
    #[error("niri ipc returned error: {0}")]
    Niri(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum NiriRequest {
    FocusedWindow,
    Windows,
    Workspaces,
    Outputs,
    EventStream,
}

#[derive(Debug)]
pub struct NiriJsonClient {
    stream: UnixStream,
    max_line_bytes: usize,
}

impl NiriJsonClient {
    pub fn connect(
        socket_path: impl AsRef<Path>,
        max_line_bytes: usize,
        timeout: Option<Duration>,
    ) -> Result<Self, NiriIpcError> {
        let stream = UnixStream::connect(socket_path)
            .map_err(|err| NiriIpcError::Connect(err.to_string()))?;
        if let Some(timeout) = timeout {
            stream
                .set_read_timeout(Some(timeout))
                .map_err(|err| NiriIpcError::Io(err.to_string()))?;
            stream
                .set_write_timeout(Some(timeout))
                .map_err(|err| NiriIpcError::Io(err.to_string()))?;
        }
        Ok(Self::from_stream(stream, max_line_bytes))
    }

    pub fn from_stream(stream: UnixStream, max_line_bytes: usize) -> Self {
        Self {
            stream,
            max_line_bytes,
        }
    }

    pub fn request<T: DeserializeOwned>(
        &mut self,
        request: &NiriRequest,
    ) -> Result<T, NiriIpcError> {
        let frame = encode_niri_request(request)?;
        self.stream
            .write_all(&frame)
            .map_err(|err| NiriIpcError::Io(err.to_string()))?;
        self.stream
            .flush()
            .map_err(|err| NiriIpcError::Io(err.to_string()))?;
        let line = read_bounded_ndjson_line(&mut self.stream, self.max_line_bytes)?;
        decode_niri_response(&line)
    }

    pub fn query_focused_window(&mut self) -> Result<Option<NiriWindow>, NiriIpcError> {
        let value: Value = self.request(&NiriRequest::FocusedWindow)?;
        let payload = value.get("FocusedWindow").cloned().unwrap_or(value);
        if payload.is_null() {
            Ok(None)
        } else {
            serde_json::from_value(payload)
                .map(Some)
                .map_err(|err| NiriIpcError::Json(err.to_string()))
        }
    }

    pub fn query_workspaces(&mut self) -> Result<Vec<NiriWorkspace>, NiriIpcError> {
        let value: Value = self.request(&NiriRequest::Workspaces)?;
        let payload = value.get("Workspaces").cloned().unwrap_or(value);
        serde_json::from_value(payload).map_err(|err| NiriIpcError::Json(err.to_string()))
    }

    pub fn read_event(&mut self) -> Result<NiriEvent, NiriIpcError> {
        let line = read_bounded_ndjson_line(&mut self.stream, self.max_line_bytes)?;
        serde_json::from_str(&line).map_err(|err| NiriIpcError::Json(err.to_string()))
    }
}

impl FocusedWindowProvider for NiriJsonClient {
    fn query_focused_window(&mut self) -> Result<Option<NiriWindow>, NiriIpcError> {
        NiriJsonClient::query_focused_window(self)
    }

    fn query_workspaces(&mut self) -> Result<Vec<NiriWorkspace>, NiriIpcError> {
        NiriJsonClient::query_workspaces(self)
    }
}

pub fn encode_niri_request(request: &NiriRequest) -> Result<Vec<u8>, NiriIpcError> {
    let mut frame =
        serde_json::to_vec(request).map_err(|err| NiriIpcError::Json(err.to_string()))?;
    frame.push(b'\n');
    Ok(frame)
}

pub fn read_bounded_ndjson_line<R: Read>(
    reader: &mut R,
    max_line_bytes: usize,
) -> Result<String, NiriIpcError> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) if line.is_empty() => return Err(NiriIpcError::Incomplete),
            Ok(0) => return Err(NiriIpcError::Incomplete),
            Ok(_) if byte[0] == b'\n' => {
                return String::from_utf8(line).map_err(|_| NiriIpcError::InvalidUtf8);
            }
            Ok(_) => {
                line.push(byte[0]);
                if line.len() > max_line_bytes {
                    return Err(NiriIpcError::FrameTooLong {
                        max: max_line_bytes,
                    });
                }
            }
            Err(err) => return Err(NiriIpcError::Io(err.to_string())),
        }
    }
}

pub fn decode_niri_response<T: DeserializeOwned>(line: &str) -> Result<T, NiriIpcError> {
    let value: Value =
        serde_json::from_str(line).map_err(|err| NiriIpcError::Json(err.to_string()))?;
    if let Some(error) = value.get("Err").or_else(|| value.get("error")) {
        return Err(NiriIpcError::Niri(error.to_string()));
    }
    let payload = value.get("Ok").cloned().unwrap_or(value);
    serde_json::from_value(payload).map_err(|err| NiriIpcError::Json(err.to_string()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NiriWindow {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<u64>,
    #[serde(
        default,
        rename = "output",
        alias = "output_name",
        alias = "output_label"
    )]
    pub output_label: Option<String>,
    #[serde(default)]
    pub is_focused: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NiriWorkspace {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(
        default,
        rename = "output",
        alias = "output_name",
        alias = "output_label"
    )]
    pub output_label: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub is_focused: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NiriEvent {
    FocusChanged { id: Option<u64> },
    WindowChanged { window: NiriWindow },
    WindowsChanged { windows: Vec<NiriWindow> },
    WindowClosed { id: Option<u64> },
    WorkspacesChanged { workspaces: Vec<NiriWorkspace> },
    WorkspaceActivated { id: Option<u64>, focused: bool },
    Unknown { type_name: Option<String> },
}

impl<'de> Deserialize<'de> for NiriEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(parse_niri_event(value))
    }
}

fn parse_niri_event(value: Value) -> NiriEvent {
    let Some(object) = value.as_object() else {
        return NiriEvent::Unknown { type_name: None };
    };
    let Some((event_name, payload)) = object.iter().next() else {
        return NiriEvent::Unknown { type_name: None };
    };
    match event_name.as_str() {
        "WindowFocusChanged" | "FocusedWindowChanged" => NiriEvent::FocusChanged {
            id: id_from_payload(payload),
        },
        "WindowOpenedOrChanged" | "WindowChanged" => parse_window_payload(payload)
            .map(|window| NiriEvent::WindowChanged { window })
            .unwrap_or_else(|| NiriEvent::Unknown {
                type_name: Some(event_name.clone()),
            }),
        "WindowsChanged" => NiriEvent::WindowsChanged {
            windows: windows_from_payload(payload),
        },
        "WindowClosed" => NiriEvent::WindowClosed {
            id: id_from_payload(payload),
        },
        "WorkspacesChanged" => NiriEvent::WorkspacesChanged {
            workspaces: workspaces_from_payload(payload),
        },
        "WorkspaceActivated" => NiriEvent::WorkspaceActivated {
            id: id_from_payload(payload),
            focused: payload
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        },
        other => NiriEvent::Unknown {
            type_name: Some(other.to_owned()),
        },
    }
}

fn id_from_payload(payload: &Value) -> Option<u64> {
    if payload.is_null() {
        return None;
    }
    payload.as_u64().or_else(|| {
        payload
            .get("id")
            .and_then(Value::as_u64)
            .or_else(|| payload.get("window_id").and_then(Value::as_u64))
    })
}

fn parse_window_payload(payload: &Value) -> Option<NiriWindow> {
    let value = payload.get("window").unwrap_or(payload).clone();
    serde_json::from_value(value).ok()
}

fn windows_from_payload(payload: &Value) -> Vec<NiriWindow> {
    let value = payload.get("windows").unwrap_or(payload).clone();
    serde_json::from_value(value).unwrap_or_default()
}

fn workspaces_from_payload(payload: &Value) -> Vec<NiriWorkspace> {
    let value = payload.get("workspaces").unwrap_or(payload).clone();
    serde_json::from_value(value).unwrap_or_default()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedWindowSnapshot {
    pub id: Option<u64>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub workspace_id: Option<u64>,
    pub output_label: Option<String>,
}

impl FocusedWindowSnapshot {
    pub fn from_window(window: &NiriWindow, workspaces: &BTreeMap<u64, NiriWorkspace>) -> Self {
        let output_label = window.output_label.clone().or_else(|| {
            window
                .workspace_id
                .and_then(|id| workspaces.get(&id))
                .and_then(|workspace| workspace.output_label.clone())
        });
        Self {
            id: window.id,
            app_id: window.app_id.clone(),
            title: window.title.clone(),
            workspace_id: window.workspace_id,
            output_label,
        }
    }

    pub fn same_target(&self, other: &FocusedWindowSnapshot) -> bool {
        match (self.id, other.id) {
            (Some(left), Some(right)) => left == right,
            _ => {
                self.app_id == other.app_id
                    && self.title == other.title
                    && self.workspace_id == other.workspace_id
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NiriStateCache {
    windows: BTreeMap<u64, NiriWindow>,
    workspaces: BTreeMap<u64, NiriWorkspace>,
    focused_window_id: Option<u64>,
    focused: Option<NiriWindow>,
    stale: bool,
}

impl NiriStateCache {
    pub fn apply_event(&mut self, event: NiriEvent) -> Option<FocusedWindowSnapshot> {
        match event {
            NiriEvent::FocusChanged { id } => {
                self.focused_window_id = id;
                self.focused = id.and_then(|id| self.windows.get(&id).cloned());
                self.stale = self.focused.is_none();
            }
            NiriEvent::WindowChanged { window } => {
                if let Some(id) = window.id {
                    if window.is_focused.unwrap_or(false) || self.focused_window_id == Some(id) {
                        self.focused_window_id = Some(id);
                        self.focused = Some(window.clone());
                        self.stale = false;
                    }
                    self.windows.insert(id, window);
                }
            }
            NiriEvent::WindowsChanged { windows } => {
                self.windows.clear();
                self.focused = None;
                for window in windows {
                    if let Some(id) = window.id {
                        if window.is_focused.unwrap_or(false) || self.focused_window_id == Some(id)
                        {
                            self.focused_window_id = Some(id);
                            self.focused = Some(window.clone());
                            self.stale = false;
                        }
                        self.windows.insert(id, window);
                    }
                }
            }
            NiriEvent::WindowClosed { id } => {
                if let Some(id) = id {
                    self.windows.remove(&id);
                    if self.focused.as_ref().and_then(|window| window.id) == Some(id) {
                        self.focused_window_id = None;
                        self.focused = None;
                        self.stale = true;
                    }
                }
            }
            NiriEvent::WorkspacesChanged { workspaces } => {
                self.workspaces = workspaces
                    .into_iter()
                    .filter_map(|workspace| workspace.id.map(|id| (id, workspace)))
                    .collect();
            }
            NiriEvent::WorkspaceActivated { .. } | NiriEvent::Unknown { .. } => {}
        }
        self.focused_window()
    }

    pub fn update_focused_window(
        &mut self,
        focused: Option<NiriWindow>,
    ) -> Option<FocusedWindowSnapshot> {
        self.focused = focused;
        self.stale = false;
        if let Some(window) = &self.focused
            && let Some(id) = window.id
        {
            self.focused_window_id = Some(id);
            self.windows.insert(id, window.clone());
        }
        self.focused_window()
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub fn is_stale(&self) -> bool {
        self.stale
    }

    pub fn focused_window(&self) -> Option<FocusedWindowSnapshot> {
        self.focused
            .as_ref()
            .map(|window| FocusedWindowSnapshot::from_window(window, &self.workspaces))
    }
}

pub trait FocusedWindowProvider {
    fn query_focused_window(&mut self) -> Result<Option<NiriWindow>, NiriIpcError>;

    fn query_workspaces(&mut self) -> Result<Vec<NiriWorkspace>, NiriIpcError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSelectionAttribution {
    pub window: Option<FocusedWindowSnapshot>,
    pub quality: AttributionQuality,
}

#[derive(Debug)]
pub struct HostClipboardAttributor<P> {
    provider: P,
    cache: NiriStateCache,
}

impl<P: FocusedWindowProvider> HostClipboardAttributor<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            cache: NiriStateCache::default(),
        }
    }

    pub fn cache_mut(&mut self) -> &mut NiriStateCache {
        &mut self.cache
    }

    pub fn on_host_selection_changed(&mut self) -> HostSelectionAttribution {
        self.cached_focused_window_guess()
    }

    pub fn cached_focused_window_guess(&mut self) -> HostSelectionAttribution {
        let quality = if self.cache.is_stale() {
            AttributionQuality::CacheStaleFocusedWindowGuess
        } else {
            AttributionQuality::FocusedWindowGuess
        };
        HostSelectionAttribution {
            window: self.cache.focused_window(),
            quality,
        }
    }

    pub fn refresh_from_provider(&mut self) -> HostSelectionAttribution {
        if let Ok(workspaces) = self.provider.query_workspaces() {
            let _ = self
                .cache
                .apply_event(NiriEvent::WorkspacesChanged { workspaces });
        }
        match self.provider.query_focused_window() {
            Ok(window) => HostSelectionAttribution {
                window: self.cache.update_focused_window(window),
                quality: AttributionQuality::FocusedWindowGuess,
            },
            Err(_) => {
                self.cache.mark_stale();
                HostSelectionAttribution {
                    window: self.cache.focused_window(),
                    quality: AttributionQuality::CacheStaleFocusedWindowGuess,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn niri_window_models_tolerate_unknown_fields() {
        let window: NiriWindow = serde_json::from_str(
            r#"{
              "id": 7,
              "app_id": "org.mozilla.firefox",
              "title": "Example",
              "workspace_id": 3,
              "future": {"ignored": true}
            }"#,
        )
        .expect("window");

        assert_eq!(window.id, Some(7));
        assert_eq!(window.app_id.as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(window.workspace_id, Some(3));
    }

    #[test]
    fn event_models_handle_focus_window_and_unknown_variants() {
        let focus: NiriEvent =
            serde_json::from_str(r#"{"WindowFocusChanged":{"id":7,"extra":true}}"#)
                .expect("focus event");
        assert_eq!(focus, NiriEvent::FocusChanged { id: Some(7) });

        let changed: NiriEvent = serde_json::from_str(
            r#"{"WindowOpenedOrChanged":{"window":{"id":7,"app_id":"foot","title":"shell"}}}"#,
        )
        .expect("window event");
        assert!(matches!(
            changed,
            NiriEvent::WindowChanged {
                window: NiriWindow { id: Some(7), .. }
            }
        ));

        let unknown: NiriEvent = serde_json::from_str(r#"{"NewFutureEvent":{"shape":"ignored"}}"#)
            .expect("unknown event");
        assert_eq!(
            unknown,
            NiriEvent::Unknown {
                type_name: Some("NewFutureEvent".to_owned())
            }
        );
    }

    #[test]
    fn cache_tracks_focus_and_resolves_output_from_workspace() {
        let mut cache = NiriStateCache::default();
        cache.apply_event(NiriEvent::WorkspacesChanged {
            workspaces: vec![NiriWorkspace {
                id: Some(3),
                output_label: Some("DP-1".to_owned()),
                ..NiriWorkspace::default()
            }],
        });
        cache.apply_event(NiriEvent::WindowsChanged {
            windows: vec![NiriWindow {
                id: Some(7),
                app_id: Some("foot".to_owned()),
                title: Some("shell".to_owned()),
                workspace_id: Some(3),
                is_focused: Some(true),
                ..NiriWindow::default()
            }],
        });

        let focused = cache.focused_window().expect("focused window");
        assert_eq!(focused.app_id.as_deref(), Some("foot"));
        assert_eq!(focused.output_label.as_deref(), Some("DP-1"));
    }

    #[test]
    fn cache_resolves_focus_event_when_window_details_arrive_later() {
        let mut cache = NiriStateCache::default();
        cache.apply_event(NiriEvent::FocusChanged { id: Some(7) });
        assert!(cache.focused_window().is_none());
        assert!(cache.is_stale());

        cache.apply_event(NiriEvent::WindowChanged {
            window: NiriWindow {
                id: Some(7),
                app_id: Some("foot".to_owned()),
                title: Some("shell".to_owned()),
                ..NiriWindow::default()
            },
        });

        let focused = cache.focused_window().expect("focused window");
        assert_eq!(focused.app_id.as_deref(), Some("foot"));
        assert!(!cache.is_stale());
    }

    #[test]
    fn direct_client_uses_unix_socket_json_and_bounded_line_reads() {
        let (client, mut server) = UnixStream::pair().expect("socketpair");
        let server_thread = thread::spawn(move || {
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                server.read_exact(&mut byte).expect("read request");
                request.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            assert_eq!(request, b"\"FocusedWindow\"\n");
            server
                .write_all(br#"{"id":7,"app_id":"foot","title":"shell","workspace_id":3}"#)
                .expect("write response");
            server.write_all(b"\n").expect("write newline");
        });
        let mut client = NiriJsonClient::from_stream(client, 256);

        let focused = client
            .query_focused_window()
            .expect("focused response")
            .expect("focused window");
        server_thread.join().expect("server thread");
        assert_eq!(focused.app_id.as_deref(), Some("foot"));
    }

    #[test]
    fn direct_client_unwraps_niri_variant_payloads() {
        let (client, mut server) = UnixStream::pair().expect("socketpair");
        let server_thread = thread::spawn(move || {
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                server.read_exact(&mut byte).expect("read request");
                request.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            assert_eq!(request, b"\"FocusedWindow\"\n");
            server
                .write_all(br#"{"Ok":{"FocusedWindow":{"id":7,"app_id":"foot","title":"shell","workspace_id":3}}}"#)
                .expect("write response");
            server.write_all(b"\n").expect("write newline");
        });
        let mut client = NiriJsonClient::from_stream(client, 256);

        let focused = client
            .query_focused_window()
            .expect("focused response")
            .expect("focused window");
        server_thread.join().expect("server thread");
        assert_eq!(focused.app_id.as_deref(), Some("foot"));
        assert_eq!(focused.workspace_id, Some(3));
    }

    #[test]
    fn direct_client_unwraps_niri_workspaces_payload() {
        let (client, mut server) = UnixStream::pair().expect("socketpair");
        let server_thread = thread::spawn(move || {
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                server.read_exact(&mut byte).expect("read request");
                request.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            assert_eq!(request, b"\"Workspaces\"\n");
            server
                .write_all(br#"{"Ok":{"Workspaces":[{"id":3,"output":"DP-3","is_focused":true}]}}"#)
                .expect("write response");
            server.write_all(b"\n").expect("write newline");
        });
        let mut client = NiriJsonClient::from_stream(client, 256);

        let workspaces = client.query_workspaces().expect("workspaces response");
        server_thread.join().expect("server thread");
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].id, Some(3));
        assert_eq!(workspaces[0].output_label.as_deref(), Some("DP-3"));
    }

    #[test]
    fn bounded_niri_line_rejects_overlong_response_before_json() {
        let mut bytes = std::io::Cursor::new(b"{not-json-but-too-long}\n".to_vec());
        let err = read_bounded_ndjson_line(&mut bytes, 4).expect_err("overlong");
        assert!(matches!(err, NiriIpcError::FrameTooLong { max: 4 }));
    }

    #[derive(Debug)]
    struct FakeFocusedWindowProvider {
        responses: Vec<Result<Option<NiriWindow>, NiriIpcError>>,
        workspace_responses: Vec<Result<Vec<NiriWorkspace>, NiriIpcError>>,
    }

    impl FocusedWindowProvider for FakeFocusedWindowProvider {
        fn query_focused_window(&mut self) -> Result<Option<NiriWindow>, NiriIpcError> {
            self.responses.remove(0)
        }

        fn query_workspaces(&mut self) -> Result<Vec<NiriWorkspace>, NiriIpcError> {
            if self.workspace_responses.is_empty() {
                Ok(Vec::new())
            } else {
                self.workspace_responses.remove(0)
            }
        }
    }

    #[test]
    fn provider_refresh_updates_focused_window() {
        let provider = FakeFocusedWindowProvider {
            responses: vec![Ok(Some(NiriWindow {
                id: Some(9),
                app_id: Some("firefox".to_owned()),
                title: Some("docs".to_owned()),
                ..NiriWindow::default()
            }))],
            workspace_responses: Vec::new(),
        };
        let mut attributor = HostClipboardAttributor::new(provider);

        let attribution = attributor.refresh_from_provider();
        assert_eq!(attribution.quality, AttributionQuality::FocusedWindowGuess);
        assert_eq!(
            attribution.window.and_then(|window| window.app_id),
            Some("firefox".to_owned())
        );
    }

    #[test]
    fn provider_refresh_resolves_output_from_queried_workspaces() {
        let provider = FakeFocusedWindowProvider {
            responses: vec![Ok(Some(NiriWindow {
                id: Some(9),
                app_id: Some("firefox".to_owned()),
                title: Some("url".to_owned()),
                workspace_id: Some(3),
                ..NiriWindow::default()
            }))],
            workspace_responses: vec![Ok(vec![NiriWorkspace {
                id: Some(3),
                output_label: Some("DP-3".to_owned()),
                ..NiriWorkspace::default()
            }])],
        };
        let mut attributor = HostClipboardAttributor::new(provider);

        let attribution = attributor.refresh_from_provider();

        assert_eq!(
            attribution
                .window
                .and_then(|window| window.output_label)
                .as_deref(),
            Some("DP-3")
        );
    }

    #[test]
    fn provider_failure_returns_cache_stale_guess() {
        let provider = FakeFocusedWindowProvider {
            responses: vec![Err(NiriIpcError::Io("disconnected".to_owned()))],
            workspace_responses: Vec::new(),
        };
        let mut attributor = HostClipboardAttributor::new(provider);
        attributor
            .cache_mut()
            .update_focused_window(Some(NiriWindow {
                id: Some(9),
                app_id: Some("firefox".to_owned()),
                ..NiriWindow::default()
            }));

        let attribution = attributor.refresh_from_provider();
        assert_eq!(
            attribution.quality,
            AttributionQuality::CacheStaleFocusedWindowGuess
        );
        assert_eq!(
            attribution.window.and_then(|window| window.app_id),
            Some("firefox".to_owned())
        );
        assert!(attributor.cache_mut().is_stale());
    }
}
