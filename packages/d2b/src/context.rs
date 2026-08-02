//! Zone selection, request bounds, and the small transport facade used by the
//! native CLI.

use std::{
    env, fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use d2b_contracts::v3::{ResourceRef, ResourceTypeName, ZoneId, identity::STANDARD_RESOURCE_TYPES};
use serde_json::{Value, json};

use crate::{CliFailure, MAX_FRAME_BYTES, SeqpacketUnixSocket, print_stdout};

/// The frozen JSON envelope version emitted by the CLI.
pub(crate) const JSON_SCHEMA_VERSION: u8 = 1;
/// The maximum lifetime admitted for a request or stream.
pub(crate) const MAX_REQUEST_LIFETIME_MS: u64 = 900_000;
/// The default deadline for one resource request.
pub(crate) const DEFAULT_REQUEST_LIFETIME_MS: u64 = 30_000;
/// The maximum bytes accepted from a caller-provided resource spec.
pub(crate) const MAX_SPEC_BYTES: usize = 64 * 1024;

/// Which output representation a command should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputMode {
    Json,
    Human,
}

impl OutputMode {
    pub(crate) const fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

/// A bounded wall-clock deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestDeadline(Duration);

impl RequestDeadline {
    pub(crate) const fn duration(self) -> Duration {
        self.0
    }
}

/// Errors raised before a resource response exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportError {
    Unavailable,
    InvalidResponse,
    OversizedResponse,
}

/// The transport boundary is deliberately injectable. Tests can provide a
/// pure response client without opening a host socket, while production uses
/// the local ComponentSession endpoint.
pub(crate) trait SessionClient: Send + Sync {
    fn invoke(&self, request: &[u8], deadline: RequestDeadline) -> Result<Vec<u8>, TransportError>;
}

#[derive(Debug)]
struct UnixSessionClient {
    socket_path: PathBuf,
}

impl SessionClient for UnixSessionClient {
    fn invoke(
        &self,
        request: &[u8],
        _deadline: RequestDeadline,
    ) -> Result<Vec<u8>, TransportError> {
        let mut socket = SeqpacketUnixSocket::connect(&self.socket_path)
            .map_err(|error| classify_transport_error(&error))?;
        let hello = crate::daemon_hello_frame("hello").map_err(|_| TransportError::Io)?;
        socket
            .send_frame(&hello)
            .map_err(|error| classify_transport_error(&error))?;
        let hello_reply = socket
            .recv_frame()
            .map_err(|error| classify_transport_error(&error))?;
        let hello_type = serde_json::from_slice::<Value>(&hello_reply)
            .ok()
            .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned));
        if hello_type.as_deref() != Some("helloOk") {
            return Err(if hello_type.as_deref() == Some("helloRejected") {
                TransportError::Unavailable
            } else {
                TransportError::InvalidResponse
            });
        }
        socket
            .send_frame(request)
            .map_err(|error| classify_transport_error(&error))?;
        let response = socket
            .recv_frame()
            .map_err(|error| classify_transport_error(&error))?;
        if response.len() > MAX_FRAME_BYTES {
            return Err(TransportError::OversizedResponse);
        }
        Ok(response)
    }
}

/// The selected Zone and its authenticated-session request facade.
pub(crate) struct ZoneContext {
    zone_name: String,
    socket_path: PathBuf,
    session_client: Arc<dyn SessionClient>,
}

impl std::fmt::Debug for ZoneContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ZoneContext")
            .field("zone_name", &self.zone_name)
            .field("session_client", &"<injected>")
            .finish()
    }
}

impl ZoneContext {
    /// Discover the nearest reachable Zone socket.
    pub(crate) fn discover(zone_arg: Option<&str>) -> Result<Self, CliFailure> {
        let requested_zone = zone_arg
            .map(str::to_owned)
            .or_else(|| env::var("D2B_ZONE").ok().filter(|value| !value.is_empty()));
        let zone_name = requested_zone.as_deref().unwrap_or("local-root").to_owned();
        validate_zone_name(&zone_name)?;

        let candidates = socket_candidates(requested_zone.as_deref());
        let socket_path = candidates
            .iter()
            .find(|candidate| candidate.exists())
            .cloned()
            .or_else(|| candidates.first().cloned())
            .ok_or_else(|| CliFailure::new(1, "zone-unavailable"))?;

        let selected_zone = requested_zone.unwrap_or_else(|| {
            socket_path
                .parent()
                .filter(|parent| parent.parent().is_some_and(|root| root.ends_with("zones")))
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or("local-root")
                .to_owned()
        });
        validate_zone_name(&selected_zone)?;

        Ok(Self {
            session_client: Arc::new(UnixSessionClient {
                socket_path: socket_path.clone(),
            }),
            zone_name: selected_zone,
            socket_path,
        })
    }

    /// Construct a context with an injected client for unit tests.
    #[cfg(test)]
    pub(crate) fn with_client(
        zone_name: impl Into<String>,
        socket_path: impl Into<PathBuf>,
        session_client: Arc<dyn SessionClient>,
    ) -> Result<Self, CliFailure> {
        let zone_name = zone_name.into();
        validate_zone_name(&zone_name)?;
        Ok(Self {
            zone_name,
            socket_path: socket_path.into(),
            session_client,
        })
    }

    pub(crate) fn zone_name(&self) -> &str {
        &self.zone_name
    }

    pub(crate) fn zone_ref(&self) -> String {
        format!("Zone/{}", self.zone_name)
    }

    #[cfg(test)]
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Admit a duration string under the one global request lifetime bound.
    pub(crate) fn deadline(value: Option<&str>) -> Result<RequestDeadline, CliFailure> {
        let duration = value
            .map(parse_duration)
            .transpose()?
            .unwrap_or_else(|| Duration::from_millis(DEFAULT_REQUEST_LIFETIME_MS));
        if duration.is_zero() || duration.as_millis() > u128::from(MAX_REQUEST_LIFETIME_MS) {
            return Err(CliFailure::new(
                2,
                "deadline must be greater than zero and no more than 900s",
            ));
        }
        Ok(RequestDeadline(duration))
    }

    /// Invoke one typed resource-plane method.
    pub(crate) fn invoke(
        &self,
        method: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        let mut request = match payload {
            Value::Object(object) => object,
            _ => {
                return Err(self.failure(
                    "internal-error",
                    "resource request payload must be an object",
                    mode,
                    1,
                ));
            }
        };
        request.insert(
            "type".to_owned(),
            Value::String("resourceRequest".to_owned()),
        );
        request.insert("method".to_owned(), Value::String(method.to_owned()));
        request.insert("zoneRef".to_owned(), Value::String(self.zone_ref()));
        request.insert(
            "schemaVersion".to_owned(),
            Value::Number(serde_json::Number::from(JSON_SCHEMA_VERSION)),
        );
        let request = serde_json::to_vec(&Value::Object(request)).map_err(|_| {
            self.failure(
                "internal-error",
                "failed to encode resource request",
                mode,
                1,
            )
        })?;

        let response = self
            .session_client
            .invoke(&request, deadline)
            .map_err(|error| self.transport_failure(error, mode))?;
        let mut value: Value = serde_json::from_slice(&response).map_err(|_| {
            self.failure(
                "exec-protocol-error",
                "Zone returned an invalid resource response",
                mode,
                1,
            )
        })?;
        if matches!(
            value.get("type").and_then(Value::as_str),
            Some("error" | "helloRejected")
        ) {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("Zone rejected the resource request");
            return Err(self.failure("not-implemented", &bounded_message(message), mode, 78));
        }
        if value
            .get("ok")
            .and_then(Value::as_bool)
            .is_some_and(|ok| !ok)
        {
            let class = value
                .get("errorClass")
                .and_then(Value::as_str)
                .unwrap_or("internal-error");
            let message = bounded_message(
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("resource request failed"),
            );
            return Err(self.failure(class, &message, mode, error_exit_code(class)));
        }
        if let Value::Object(object) = &mut value {
            object.entry("ok".to_owned()).or_insert(Value::Bool(true));
            object.insert("zoneRef".to_owned(), Value::String(self.zone_ref()));
            object.insert(
                "schemaVersion".to_owned(),
                Value::Number(serde_json::Number::from(JSON_SCHEMA_VERSION)),
            );
        }
        Ok(value)
    }

    pub(crate) fn failure(
        &self,
        error_class: &str,
        message: &str,
        mode: OutputMode,
        exit_code: i32,
    ) -> CliFailure {
        let message = bounded_message(message);
        let mut failure = CliFailure::new(exit_code, format!("{error_class}: {message}"));
        if mode.is_json() {
            let envelope = json!({
                "ok": false,
                "zoneRef": self.zone_ref(),
                "errorClass": error_class,
                "message": message,
                "schemaVersion": JSON_SCHEMA_VERSION,
            });
            if let Ok(mut rendered) = serde_json::to_string(&envelope) {
                rendered.push('\n');
                failure.rendered_stderr = Some(rendered);
            }
        }
        failure
    }

    fn transport_failure(&self, error: TransportError, mode: OutputMode) -> CliFailure {
        match error {
            TransportError::Unavailable | TransportError::Io => {
                self.failure("zone-unavailable", "Zone runtime is unavailable", mode, 1)
            }
            TransportError::InvalidResponse => self.failure(
                "exec-protocol-error",
                "Zone returned an invalid resource response",
                mode,
                1,
            ),
            TransportError::OversizedResponse => self.failure(
                "resource-schema-invalid",
                "Zone response exceeded the bounded response size",
                mode,
                1,
            ),
        }
    }

    /// Emit a complete response using the selected output mode.
    pub(crate) fn emit(&self, value: &Value, mode: OutputMode) -> Result<(), CliFailure> {
        match mode {
            OutputMode::Json => {
                let mut rendered = serde_json::to_string_pretty(value).map_err(|_| {
                    self.failure("internal-error", "failed to render JSON", mode, 1)
                })?;
                rendered.push('\n');
                print_stdout(&rendered);
            }
            OutputMode::Human => {
                let summary = human_summary(value);
                print_stdout(&summary);
                print_stdout("\n");
            }
        }
        Ok(())
    }

    pub(crate) fn emit_stream(&self, value: &Value, mode: OutputMode) -> Result<(), CliFailure> {
        if !mode.is_json() {
            return Err(self.failure("ref-invalid", "watch output is JSON-lines only", mode, 2));
        }
        let events = value
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]);
        for event in events {
            let mut rendered = serde_json::to_string(&event).map_err(|_| {
                self.failure("internal-error", "failed to render watch event", mode, 1)
            })?;
            rendered.push('\n');
            print_stdout(&rendered);
        }
        Ok(())
    }
}

pub(crate) fn output_mode(json_flag: bool, human_flag: bool) -> Result<OutputMode, CliFailure> {
    if json_flag && human_flag {
        return Err(CliFailure::new(
            2,
            "--json and --human are mutually exclusive",
        ));
    }
    if json_flag || (!human_flag && !crate::stdout_is_tty()) {
        Ok(OutputMode::Json)
    } else {
        Ok(OutputMode::Human)
    }
}

pub(crate) fn parse_resource_ref(
    value: &str,
    default_type: Option<&str>,
) -> Result<ResourceRef, CliFailure> {
    let canonical = if value.contains('/') {
        value.to_owned()
    } else {
        let resource_type = default_type.ok_or_else(|| {
            CliFailure::new(2, "resource reference must use <ResourceType>/<name>")
        })?;
        format!("{resource_type}/{value}")
    };
    ResourceRef::parse(&canonical)
        .map_err(|_| CliFailure::new(2, "ref-invalid: invalid ResourceRef"))
}

pub(crate) fn parse_resource_type(value: &str) -> Result<ResourceTypeName, CliFailure> {
    ResourceTypeName::parse(value.to_owned())
        .map_err(|_| CliFailure::new(2, "ref-invalid: unknown ResourceType"))
}

pub(crate) fn standard_resource_types() -> &'static [&'static str; 19] {
    &STANDARD_RESOURCE_TYPES
}

pub(crate) fn read_spec(spec_file: Option<&Path>, spec_stdin: bool) -> Result<Value, CliFailure> {
    if spec_file.is_some() == spec_stdin {
        return Err(CliFailure::new(
            2,
            "exactly one of --spec-file or --spec-stdin is required",
        ));
    }
    let bytes = if let Some(path) = spec_file {
        fs::read(path).map_err(|_| CliFailure::new(1, "failed to read resource spec"))?
    } else {
        let mut bytes = Vec::new();
        io::stdin()
            .lock()
            .take((MAX_SPEC_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| CliFailure::new(1, "failed to read resource spec from stdin"))?;
        bytes
    };
    if bytes.len() > MAX_SPEC_BYTES {
        return Err(CliFailure::new(2, "resource spec exceeds the 64 KiB bound"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CliFailure::new(2, "resource-schema-invalid: spec must be JSON"))
}

pub(crate) fn bounded_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(4096)
        .collect()
}

fn socket_candidates(requested_zone: Option<&str>) -> Vec<PathBuf> {
    if let Some(path) = env::var_os("D2B_PUBLIC_SOCKET") {
        return vec![PathBuf::from(path)];
    }

    if let Some(zone) = requested_zone {
        return vec![PathBuf::from(format!("/run/d2b/zones/{zone}/public.sock"))];
    }

    let mut candidates = Vec::new();
    let zone_root = Path::new("/run/d2b/zones");
    if let Ok(entries) = fs::read_dir(zone_root) {
        let mut zone_paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("public.sock"))
            .collect();
        zone_paths.sort();
        candidates.extend(zone_paths);
    }
    candidates.push(PathBuf::from("/run/d2b/public.sock"));
    candidates
}

fn validate_zone_name(value: &str) -> Result<(), CliFailure> {
    ZoneId::parse(value.to_owned())
        .map(|_| ())
        .map_err(|_| CliFailure::new(2, "ref-invalid: invalid Zone name"))
}

fn parse_duration(value: &str) -> Result<Duration, CliFailure> {
    let (number, suffix) = value.trim().split_at(
        value
            .trim()
            .trim_end_matches(|character: char| character.is_ascii_alphabetic())
            .len(),
    );
    let amount: u64 = number
        .parse()
        .map_err(|_| CliFailure::new(2, "deadline must use a duration such as 30s or 5m"))?;
    let millis = match suffix {
        "ms" => amount,
        "s" => amount.saturating_mul(1_000),
        "m" => amount.saturating_mul(60_000),
        "h" => amount.saturating_mul(3_600_000),
        _ => {
            return Err(CliFailure::new(2, "deadline must use ms, s, m, or h"));
        }
    };
    Ok(Duration::from_millis(millis))
}

fn classify_transport_error(error: &io::Error) -> TransportError {
    match error.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => TransportError::Unavailable,
        io::ErrorKind::InvalidData => TransportError::InvalidResponse,
        _ => TransportError::Io,
    }
}

fn error_exit_code(class: &str) -> i32 {
    match class {
        "ref-invalid" | "resource-schema-invalid" => 2,
        "operation-cancelled" => 3,
        _ => 1,
    }
}

fn human_summary(value: &Value) -> String {
    if let Some(object) = value.as_object() {
        if let Some(resource_ref) = object.get("resourceRef").and_then(Value::as_str) {
            let phase = object
                .get("status")
                .and_then(Value::as_object)
                .and_then(|status| status.get("phase"))
                .and_then(Value::as_str)
                .or_else(|| object.get("phase").and_then(Value::as_str))
                .unwrap_or("unknown");
            let posture = object
                .pointer("/status/isolationPosture")
                .and_then(Value::as_str)
                .or_else(|| object.get("isolationPosture").and_then(Value::as_str));
            let posture = if posture == Some("none") {
                " [no isolation]"
            } else {
                ""
            };
            return format!("{resource_ref}\t{phase}{posture}");
        }
        if let Some(items) = object.get("items").and_then(Value::as_array) {
            let mut output = String::from("RESOURCE\tPHASE");
            for item in items {
                let resource_ref = item
                    .get("resourceRef")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| {
                        let resource_type = item.get("type").and_then(Value::as_str)?;
                        let name = item.pointer("/metadata/name").and_then(Value::as_str)?;
                        Some(format!("{resource_type}/{name}"))
                    })
                    .unwrap_or_else(|| "<unknown>".to_owned());
                let phase = item
                    .pointer("/status/phase")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let posture = item
                    .pointer("/status/isolationPosture")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("isolationPosture").and_then(Value::as_str));
                let posture = if posture == Some("none") {
                    " [no isolation]"
                } else {
                    ""
                };
                output.push_str(&format!("\n{resource_ref}\t{phase}{posture}"));
            }
            return output;
        }
        if let Some(class) = object.get("errorClass").and_then(Value::as_str) {
            return format!(
                "{class}: {}",
                object
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed")
            );
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockClient {
        requests: Mutex<Vec<Vec<u8>>>,
        response: Vec<u8>,
    }

    impl SessionClient for MockClient {
        fn invoke(
            &self,
            request: &[u8],
            _deadline: RequestDeadline,
        ) -> Result<Vec<u8>, TransportError> {
            self.requests.lock().unwrap().push(request.to_vec());
            Ok(self.response.clone())
        }
    }

    #[test]
    fn resource_refs_use_only_explicit_default_types() {
        assert_eq!(
            parse_resource_ref("work", Some("Guest"))
                .unwrap()
                .to_canonical_string(),
            "Guest/work"
        );
        assert!(parse_resource_ref("work", None).is_err());
        assert!(parse_resource_ref("Widget/work", None).is_err());
        assert_eq!(
            parse_resource_ref("Endpoint/ready", None)
                .unwrap()
                .to_canonical_string(),
            "Endpoint/ready"
        );
        assert_eq!(
            parse_resource_ref("ResourceImport/mic", None)
                .unwrap()
                .to_canonical_string(),
            "ResourceImport/mic"
        );
    }

    #[test]
    fn deadline_is_capped_at_nine_hundred_seconds() {
        assert_eq!(
            ZoneContext::deadline(Some("900s")).unwrap().duration(),
            Duration::from_secs(900)
        );
        assert!(ZoneContext::deadline(Some("901s")).is_err());
        assert!(ZoneContext::deadline(Some("0s")).is_err());
        assert!(ZoneContext::deadline(Some("30x")).is_err());
    }

    #[test]
    fn injected_context_adds_frozen_envelope_fields() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"items":[]}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/zones/dev/public.sock", client.clone())
                .unwrap();
        let response = context
            .invoke(
                "List",
                json!({"resourceType":"Guest"}),
                ZoneContext::deadline(None).unwrap(),
                OutputMode::Json,
            )
            .unwrap();
        assert_eq!(response["schemaVersion"], 1);
        assert_eq!(response["zoneRef"], "Zone/dev");
        assert_eq!(response["ok"], true);
        let request = client.requests.lock().unwrap();
        let request: Value = serde_json::from_slice(&request[0]).unwrap();
        assert_eq!(request["method"], "List");
        assert_eq!(request["zoneRef"], "Zone/dev");
    }

    #[test]
    fn direct_socket_overrides_do_not_infer_a_zone_from_an_arbitrary_temp_path() {
        let candidates = socket_candidates(Some("dev"));
        assert_eq!(
            candidates,
            vec![PathBuf::from("/run/d2b/zones/dev/public.sock")]
        );
        let socket = PathBuf::from("/tmp/test-public.sock");
        let inferred = socket
            .parent()
            .filter(|parent| parent.parent().is_some_and(|root| root.ends_with("zones")))
            .and_then(Path::file_name);
        assert!(inferred.is_none());
    }

    #[test]
    fn human_host_summary_marks_the_no_isolation_posture() {
        let summary = human_summary(&json!({
            "resourceRef": "Host/alice",
            "status": {
                "phase": "Ready",
                "isolationPosture": "none"
            }
        }));
        assert!(summary.contains("[no isolation]"));
    }
}
