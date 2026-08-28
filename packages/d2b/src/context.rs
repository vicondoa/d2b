//! Zone selection, request bounds, and the small transport facade used by the
//! native CLI.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    future::{Future, ready},
    io::{self, IoSliceMut, Read as _},
    os::fd::{AsRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};

use d2b_contracts::{
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected,
    KnownFeatureFlag, SemverRange,
};
use d2b_contracts_control::public_wire::{
    ExecReadOutputResult, ExecStream, ExecWriteStdinResult, NamedProcessStreamErrorKind,
    NamedProcessStreamRequest, NamedProcessStreamRequestFrame, NamedProcessStreamResponse,
    NamedProcessStreamResponseFrame,
};
use d2b_core::{
    bundle::Bundle,
    bundle_resolver::HostRuntime,
    closures::ClosureMetadata,
    host::HostJson,
    processes::ProcessesJson,
};
use d2b_contracts_resource::v3::identity::STANDARD_RESOURCE_TYPES;
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceErrorKind, ResourceRef, ResourceTypeName, RetryClass, ZoneId,
};
use d2b_resource_client::{
    AssignmentIdentity, CallOptions, CancellationToken, ClientError, ConnectedSession,
    ConnectedZoneSession, MetadataInput, NamedStreamTransport, ProcessAttachClient,
    ProcessAttachOpenRequest, ProcessAttachOptions, ProcessAttachTarget, ResourceCallOptions,
    ResourceVerb, RetryPolicy, RouteRecord, RouteTable, ScopedResourceMutation, ServiceOwner,
    SystemClock, TargetInput, TerminalSize, TransportKind, TransportSelection, WallClock,
    ZoneClient, ZonePeerIdentity, ZoneServiceKind, ZoneSessionConnector, ZoneSessionPin,
    ZoneSocketConnector, resource_verb_is_mutating,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use nix::sys::socket::{
    AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, connect, send, socket,
};
use rustix::net::sockopt::{Timeout as SocketTimeout, set_socket_timeout};
use rustix::net::{RecvAncillaryBuffer, RecvFlags, recvmsg};

use crate::terminal_client::TerminalHostIo;
use crate::{CliFailure, MAX_FRAME_BYTES, print_stdout};

/// The frozen JSON envelope version emitted by the CLI.
pub(crate) const JSON_SCHEMA_VERSION: u8 = 1;
/// The maximum lifetime admitted for a request or stream.
pub(crate) const MAX_REQUEST_LIFETIME_MS: u64 = 900_000;
pub(crate) const LOCAL_HANDSHAKE_DEADLINE_MS: u64 = 5_000;
/// The default deadline for one resource request.
pub(crate) const DEFAULT_REQUEST_LIFETIME_MS: u64 = 30_000;
pub(crate) const MAX_EXPEDITED_DEADLINE_MS: u64 = 10_000;
/// The maximum bytes accepted from a caller-provided resource spec.
pub(crate) const MAX_SPEC_BYTES: usize = 64 * 1024;

pub(crate) const DEFAULT_MANIFEST_PATH: &str = "/run/current-system/sw/share/d2b/vms.json";
pub(crate) const DEFAULT_BUNDLE_PATH: &str = "/etc/d2b/bundle.json";
pub(crate) const DEFAULT_PUBLIC_SOCKET: &str = d2b_contracts::PUBLIC_SOCKET_PATH;
pub(crate) const DEFAULT_BROKER_SOCKET: &str = d2b_contracts::BROKER_SOCKET_PATH;
pub(crate) const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/d2b/runtime/host-runtime.json";
pub(crate) const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
pub(crate) const RUNTIME_UNKNOWN: &str = "unknown";
pub(crate) const SYSTEM_TOOL_PATH: &str =
    "/run/current-system/sw/bin:/usr/bin:/usr/sbin:/bin:/sbin";
pub(crate) const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/d2b/daemon-state";
pub(crate) const DEFAULT_METRICS_URL: &str = "";

pub(crate) fn system_tool_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.env("PATH", SYSTEM_TOOL_PATH);
    command
}

#[derive(Debug, Clone)]
pub(crate) struct CliContext {
    pub(crate) manifest_path: PathBuf,
    pub(crate) bundle_path: PathBuf,
    pub(crate) public_socket: PathBuf,
    pub(crate) broker_socket: PathBuf,
    pub(crate) state_root: Option<PathBuf>,
    pub(crate) host_runtime_path: PathBuf,
    pub(crate) system_state_fixture: Option<SystemStateFixture>,
    pub(crate) auth_status_fixture: Option<AuthStatusFixture>,
    pub(crate) daemon_state_dir: PathBuf,
    pub(crate) metrics_url: String,
}

impl CliContext {
    pub(crate) fn from_env() -> Result<Self, CliFailure> {
        Ok(Self {
            manifest_path: env_path("D2B_MANIFEST_PATH", DEFAULT_MANIFEST_PATH),
            bundle_path: env_path("D2B_BUNDLE_PATH", DEFAULT_BUNDLE_PATH),
            public_socket: env_path("D2B_PUBLIC_SOCKET", DEFAULT_PUBLIC_SOCKET),
            broker_socket: env_path("D2B_BROKER_SOCKET", DEFAULT_BROKER_SOCKET),
            state_root: env::var_os("D2B_STATE_ROOT").map(PathBuf::from),
            host_runtime_path: env_path("D2B_HOST_RUNTIME_PATH", DEFAULT_HOST_RUNTIME_PATH),
            system_state_fixture: maybe_load_json_env("D2B_TEST_SYSTEM_STATE_JSON")?,
            auth_status_fixture: maybe_load_json_env("D2B_AUTH_STATUS_FIXTURE")?,
            daemon_state_dir: env_path("D2B_DAEMON_STATE_DIR", DEFAULT_DAEMON_STATE_DIR),
            metrics_url: env::var("D2B_METRICS_URL")
                .unwrap_or_else(|_| DEFAULT_METRICS_URL.to_owned()),
        })
    }

    pub(crate) fn load_manifest(&self) -> Result<ManifestDocument, CliFailure> {
        read_json_file(&self.manifest_path).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to read {}: {err}", self.manifest_path.display()),
            )
        })
    }

    pub(crate) fn load_bundle_context(&self) -> Result<Option<BundleContext>, CliFailure> {
        match self.bundle_path.try_exists() {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(err) => {
                return Err(CliFailure::new(
                    1,
                    format!("failed to inspect {}: {err}", self.bundle_path.display()),
                ));
            }
        }
        let bundle: Bundle = read_json_file(&self.bundle_path).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to read {}: {err}", self.bundle_path.display()),
            )
        })?;
        let base_dir = self
            .bundle_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        let host = read_bundle_json::<HostJson>(&base_dir, &bundle.host_path)?;
        let processes = read_bundle_json::<ProcessesJson>(&base_dir, &bundle.processes_path)?;
        let mut closures = BTreeMap::new();
        for closure_ref in &bundle.closures {
            if let Some(closure) =
                read_bundle_json::<ClosureMetadata>(&base_dir, &closure_ref.path)?
            {
                closures.insert(closure_ref.vm.clone(), closure);
            }
        }
        let host_runtime = if self.host_runtime_path.exists() {
            read_json_file::<HostRuntime>(&self.host_runtime_path).ok()
        } else {
            None
        };
        Ok(Some(BundleContext {
            host,
            processes,
            closures,
            host_runtime,
        }))
    }
}

#[derive(Debug)]
pub(crate) struct BundleContext {
    pub(crate) host: Option<HostJson>,
    pub(crate) processes: Option<ProcessesJson>,
    pub(crate) closures: BTreeMap<String, ClosureMetadata>,
    pub(crate) host_runtime: Option<HostRuntime>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManifestDocument {
    #[serde(rename = "_manifest", default)]
    _manifest: Option<Value>,
    #[serde(rename = "_observability", default)]
    _observability: Option<Value>,
    #[serde(flatten)]
    pub(crate) entries: BTreeMap<String, ManifestVm>,
}

impl ManifestDocument {
    pub(crate) fn vms(&self) -> Vec<&ManifestVm> {
        self.entries
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(_, vm)| vm)
            .collect()
    }

    pub(crate) fn get_vm(&self, name: &str) -> Option<&ManifestVm> {
        self.entries.get(name).filter(|_| !name.starts_with('_'))
    }

    pub(crate) fn bridge_names(&self) -> BTreeSet<String> {
        self.vms()
            .iter()
            .map(|vm| vm.bridge.clone())
            .collect::<BTreeSet<_>>()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestVm {
    pub(crate) name: String,
    pub(crate) env: Option<String>,
    pub(crate) graphics: bool,
    pub(crate) tpm: bool,
    pub(crate) audio: bool,
    pub(crate) usbip_yubikey: bool,
    pub(crate) static_ip: Option<String>,
    pub(crate) is_net_vm: bool,
    pub(crate) state_dir: String,
    pub(crate) bridge: String,
    pub(crate) ssh_user: Option<String>,
    pub(crate) runtime: Option<ManifestRuntime>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestRuntime {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) capabilities: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct SystemStateFixture {
    pub(crate) units: BTreeMap<String, String>,
    pub(crate) bridges: BTreeMap<String, BridgeHealthFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BridgeHealthFixture {
    pub(crate) state: String,
    pub(crate) admin: String,
    pub(crate) expected_carrier: String,
    pub(crate) result: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct AuthStatusFixture {
    pub(crate) public_reachable: Option<bool>,
    pub(crate) public_version: Option<String>,
    pub(crate) broker_reachable: Option<bool>,
    pub(crate) broker_version: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SocketProbe {
    pub(crate) reachable: bool,
    pub(crate) version: Option<String>,
}

pub(crate) fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

pub(crate) fn maybe_load_json_env<T>(name: &str) -> Result<Option<T>, CliFailure>
where
    T: for<'de> Deserialize<'de>,
{
    match env::var_os(name) {
        Some(path) => read_json_file::<T>(&PathBuf::from(path))
            .map(Some)
            .map_err(|err| CliFailure::new(1, format!("failed to read {name}: {err}"))),
        None => Ok(None),
    }
}

pub(crate) fn read_json_file<T>(path: &Path) -> Result<T, io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    let data = fs::read(path)?;
    serde_json::from_slice(&data).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(crate) fn read_bundle_json<T>(base_dir: &Path, raw_path: &str) -> Result<Option<T>, CliFailure>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = Path::new(raw_path);
    let path = if raw.is_absolute() && raw.exists() {
        raw.to_path_buf()
    } else if raw.is_absolute() {
        raw.file_name()
            .map(|name| base_dir.join(name))
            .unwrap_or_else(|| raw.to_path_buf())
    } else {
        base_dir.join(raw)
    };
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(&path)
        .map(Some)
        .map_err(|err| CliFailure::new(1, format!("failed to read {}: {err}", path.display())))
}

pub(crate) fn read_symlink_target(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|target| target.display().to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    pub(crate) payload: IpcHelloOk,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: IpcHelloRejected,
    pub(crate) error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ErrorFrame {
    #[serde(rename = "type")]
    _type_name: String,
    pub(crate) error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonErrorEnvelope {
    pub(crate) kind: String,
    #[serde(alias = "exitCode", alias = "code")]
    pub(crate) exit_code: u8,
    pub(crate) message: String,
    pub(crate) remediation: String,
}

pub(crate) fn encode_type_tagged_message<T>(
    type_name: &str,
    message: &T,
    context: &str,
) -> Result<Vec<u8>, CliFailure>
where
    T: Serialize,
{
    let mut value = serde_json::to_value(message)
        .map_err(|err| CliFailure::new(1, format!("failed to encode {context}: {err}")))?;
    value
        .as_object_mut()
        .ok_or_else(|| {
            CliFailure::new(
                1,
                format!("failed to encode {context}: JSON object required"),
            )
        })?
        .insert("type".to_owned(), Value::String(type_name.to_owned()));
    serde_json::to_vec(&value)
        .map_err(|err| CliFailure::new(1, format!("failed to encode {context}: {err}")))
}

pub(crate) fn daemon_supported_features() -> Vec<d2b_contracts::FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::StatusCheckBridges.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
        KnownFeatureFlag::ConfiguredLaunchV1.wire_value(),
        KnownFeatureFlag::UnsafeLocalProviderV1.wire_value(),
    ]
}

pub(crate) fn daemon_hello_frame(type_name: &str) -> Result<Vec<u8>, CliFailure> {
    let hello = IpcHello {
        client_version: SemverRange::new(DEFAULT_CLIENT_VERSION_RANGE).map_err(|err| {
            CliFailure::new(1, format!("failed to build hello version range: {err}"))
        })?,
        supported_features: daemon_supported_features(),
    };
    encode_type_tagged_message(type_name, &hello, "hello request")
}

pub(crate) fn decode_daemon_frame(response: &[u8], context: &str) -> Result<Value, CliFailure> {
    serde_json::from_slice(response)
        .map_err(|err| CliFailure::new(1, format!("failed to decode {context}: {err}")))
}

pub(crate) fn cli_failure_from_daemon_error(error: DaemonErrorEnvelope) -> CliFailure {
    let message = if error.remediation.is_empty() {
        format!("{}: {}", error.kind, error.message)
    } else {
        format!("{}: {} ({})", error.kind, error.message, error.remediation)
    };
    CliFailure::new(i32::from(error.exit_code), message)
}

pub(crate) fn parse_hello_reply(response: &[u8]) -> Result<IpcHelloOk, CliFailure> {
    let value = decode_daemon_frame(response, "hello reply")?;
    let Some(type_name) = value.get("type").and_then(Value::as_str) else {
        return Err(CliFailure::new(
            1,
            "daemon hello reply was missing a type discriminator",
        ));
    };
    match type_name {
        "helloOk" => serde_json::from_value::<HelloOkFrame>(value)
            .map(|frame| frame.payload)
            .map_err(|err| CliFailure::new(1, format!("failed to decode helloOk reply: {err}"))),
        "helloRejected" => {
            let frame: HelloRejectedFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode helloRejected reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        "error" => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode error reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected hello reply type {other}"),
        )),
    }
}

pub(crate) fn is_daemon_unreachable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub(crate) fn probe_socket(path: &Path) -> Result<SocketProbe, CliFailure> {
    let mut socket = SeqpacketUnixSocket::connect(path).map_err(|err| {
        CliFailure::new(1, format!("failed to connect to {}: {err}", path.display()))
    })?;
    let payload = daemon_hello_frame("hello")?;
    socket
        .send_frame(&payload)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let hello = parse_hello_reply(&response)?;
    Ok(SocketProbe {
        reachable: true,
        version: Some(hello.selected_version.as_str().to_owned()),
    })
}

pub(crate) struct SeqpacketUnixSocket {
    fd: OwnedFd,
}

impl SeqpacketUnixSocket {
    #[cfg(test)]
    pub(crate) fn from_owned_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    pub(crate) fn connect(path: &Path) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .map_err(nix_err_to_io)?;
        let addr = UnixAddr::new(path).map_err(nix_err_to_io)?;
        connect(fd.as_raw_fd(), &addr).map_err(nix_err_to_io)?;
        Ok(Self { fd })
    }

    pub(crate) fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds 1 MiB limit",
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let sent = send(self.fd.as_raw_fd(), &frame, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }

    pub(crate) fn set_io_timeout(&self, timeout: Duration) -> io::Result<()> {
        set_socket_timeout(&self.fd, SocketTimeout::Recv, Some(timeout))
            .map_err(io::Error::from)?;
        set_socket_timeout(&self.fd, SocketTimeout::Send, Some(timeout))
            .map_err(io::Error::from)?;
        Ok(())
    }

    pub(crate) fn recv_frame(&mut self) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + 4];
        let mut iov = [IoSliceMut::new(&mut buffer)];
        let mut ancillary_bytes = [0_u8; rustix::cmsg_space!(ScmRights(32))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut ancillary_bytes);
        let received = recvmsg(&self.fd, &mut iov, &mut ancillary, RecvFlags::empty())
            .map_err(io::Error::from)?;
        if received.flags.contains(RecvFlags::TRUNC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized seqpacket frame",
            ));
        }
        if received.bytes < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_BYTES || expected + 4 != received.bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        if ancillary.drain().next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ancillary data is not permitted on the CLI transport",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }
}

pub(crate) fn nix_err_to_io(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

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

    pub(crate) fn remaining(self, elapsed: Duration) -> Option<Self> {
        self.0
            .checked_sub(elapsed)
            .filter(|value| !value.is_zero())
            .map(Self)
    }
}

/// Errors raised by the test-only injected transport.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportError {
    Unavailable,
    InvalidResponse,
    OversizedResponse,
    AncillaryData,
    DeadlineExceeded,
    AuthRejected,
    Io,
}

/// The transport boundary is deliberately injectable in unit tests. Production
/// uses the typed `d2b-resource-client` facade and its private Zone adapter.
#[cfg(test)]
pub(crate) trait SessionClient: Send + Sync {
    fn invoke(&self, request: &[u8], deadline: RequestDeadline) -> Result<Vec<u8>, TransportError>;
}

#[derive(Clone)]
struct CanonicalZoneBackend {
    zone_name: String,
    zone_path: d2b_contracts_zone_session::v3::zone_routing::ZonePath,
    socket_path: PathBuf,
}

impl std::fmt::Debug for CanonicalZoneBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalZoneBackend")
            .field("zone_name", &self.zone_name)
            .field("session", &"<authenticated>")
            .finish()
    }
}

struct ContextBackend {
    canonical: CanonicalZoneBackend,
    #[cfg(test)]
    injected: Option<Arc<dyn SessionClient>>,
}

impl std::fmt::Debug for ContextBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(test)]
        if self.injected.is_some() {
            return formatter.write_str("Injected(<test>)");
        }
        self.canonical.fmt(formatter)
    }
}

/// The selected Zone and its authenticated-session request facade.
pub(crate) struct ZoneContext {
    zone_name: String,
    explicit_zone: bool,
    socket_path: PathBuf,
    zone_path: d2b_contracts_zone_session::v3::zone_routing::ZonePath,
    backend: ContextBackend,
}

impl std::fmt::Debug for ZoneContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ZoneContext")
            .field("zone_name", &self.zone_name)
            .field("explicit_zone", &self.explicit_zone)
            .field("backend", &self.backend)
            .finish()
    }
}

impl ZoneContext {
    pub(crate) fn local_only() -> Self {
        let socket_path = env::var_os("D2B_PUBLIC_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/d2b/public.sock"));
        Self::from_socket("local-root".to_owned(), socket_path)
    }

    pub(crate) fn local_only_with_explicit_zone(explicit_zone: bool) -> Self {
        let mut context = Self::local_only();
        context.explicit_zone = explicit_zone;
        context
    }

    /// Select the root public listener and an optional Zone routing target.
    pub(crate) fn discover(zone_arg: Option<&str>) -> Result<Self, CliFailure> {
        let requested_zone = zone_arg
            .map(str::to_owned)
            .or_else(|| env::var("D2B_ZONE").ok().filter(|value| !value.is_empty()));
        let explicit_zone = requested_zone.is_some();
        let zone_name = requested_zone.as_deref().unwrap_or("local-root").to_owned();
        validate_zone_name(&zone_name)?;

        let direct_override = env::var_os("D2B_PUBLIC_SOCKET").is_some();
        let socket_path = env::var_os("D2B_PUBLIC_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/d2b/public.sock"));
        if !direct_override && !socket_reachable(&socket_path) {
            return Err(CliFailure::new(1, "zone-unavailable"));
        }

        let selected_zone = requested_zone.unwrap_or_else(|| "local-root".to_owned());
        validate_zone_name(&selected_zone)?;

        let zone_path = zone_path(&selected_zone)
            .map_err(|_| CliFailure::new(2, "ref-invalid: invalid Zone name"))?;
        let backend = canonical_backend(&selected_zone, &socket_path)?;
        Ok(Self {
            zone_name: selected_zone,
            explicit_zone,
            socket_path,
            zone_path,
            backend,
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
        let socket_path = socket_path.into();
        let zone_path = zone_path(&zone_name)
            .map_err(|_| CliFailure::new(2, "ref-invalid: invalid Zone name"))?;
        let mut backend = canonical_backend(&zone_name, &socket_path)?;
        backend.injected = Some(session_client);
        Ok(Self {
            zone_name,
            explicit_zone: false,
            socket_path,
            zone_path,
            backend,
        })
    }

    fn from_socket(zone_name: String, socket_path: PathBuf) -> Self {
        let zone_path = zone_path(&zone_name).expect("validated local Zone name");
        let backend = canonical_backend(&zone_name, &socket_path)
            .expect("validated local Zone socket backend");
        Self {
            zone_name,
            explicit_zone: false,
            socket_path,
            zone_path,
            backend,
        }
    }

    pub(crate) fn zone_name(&self) -> &str {
        &self.zone_name
    }

    pub(crate) const fn has_explicit_zone(&self) -> bool {
        self.explicit_zone
    }

    pub(crate) fn zone_ref(&self) -> String {
        format!("Zone/{}", self.zone_name)
    }

    pub(crate) fn public_socket_path(&self) -> &Path {
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

    pub(crate) fn expedited_deadline(value: Option<&str>) -> Result<Option<u64>, CliFailure> {
        let Some(value) = value else {
            return Ok(None);
        };
        let duration = parse_duration(value)?;
        let millis = duration.as_millis();
        if millis == 0 || millis > u128::from(MAX_EXPEDITED_DEADLINE_MS) {
            return Err(CliFailure::new(
                2,
                "reconcile deadline must be greater than zero and no more than 10s",
            ));
        }
        Ok(Some(millis as u64))
    }

    /// Invoke one typed resource-plane method.
    pub(crate) fn invoke(
        &self,
        method: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        self.invoke_with_verb(
            method,
            payload,
            deadline,
            mode,
            resource_verb(method, false),
        )
    }

    /// Invoke one typed resource-plane mutation with an explicit mutating verb.
    pub(crate) fn invoke_mutating(
        &self,
        method: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        self.invoke_with_verb(method, payload, deadline, mode, resource_verb(method, true))
    }

    fn invoke_with_verb(
        &self,
        method: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
        verb: ResourceVerb,
    ) -> Result<Value, CliFailure> {
        #[cfg(test)]
        if let Some(client) = &self.backend.injected {
            return self.invoke_injected(
                client.as_ref(),
                method,
                payload,
                deadline,
                mode,
                None,
                None,
            );
        }

        let value = self
            .backend
            .canonical
            .invoke_with_verb(
                method,
                payload,
                deadline,
                operation_service(method),
                None,
                verb,
            )
            .map_err(|error| self.client_failure(error, mode))?;
        self.decorate_response(value)
    }

    /// Invoke one exact non-resource Zone service operation.
    ///
    /// Diagnostic operations are still routed through the typed Zone client.
    /// The service and session verb are bound into the authenticated session
    /// request rather than inferred from a user-provided resource verb.
    pub(crate) fn invoke_service(
        &self,
        service: ZoneServiceKind,
        operation: &str,
        session_verb: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        #[cfg(test)]
        if let Some(client) = &self.backend.injected {
            return self.invoke_injected(
                client.as_ref(),
                operation,
                payload,
                deadline,
                mode,
                Some(service),
                Some(session_verb),
            );
        }

        let value = self
            .backend
            .canonical
            .invoke_service(operation, payload, deadline, service, Some(session_verb))
            .map_err(|error| self.client_failure(error, mode))?;
        self.decorate_response(value)
    }

    /// Proxy one typed Process or ShellSession attachment through the Zone
    /// session. Shells retain the same authenticated session pin while their
    /// named stream is driven by the terminal adapter below.
    pub(crate) fn attach_process(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        #[cfg(test)]
        let result = self.backend.canonical.attach_process(
            resource_ref.clone(),
            interactive,
            tty,
            deadline,
            self.backend.injected.clone(),
        );
        #[cfg(not(test))]
        let result =
            self.backend
                .canonical
                .attach_process(resource_ref.clone(), interactive, tty, deadline);
        result.map_err(|error| self.client_failure(error, mode))?;
        self.decorate_response(json!({
            "attached": true,
            "interactive": interactive,
            "resourceRef": resource_ref.to_canonical_string(),
            "tty": tty,
        }))
    }

    pub(crate) fn attach_shell(
        &self,
        session_ref: ResourceRef,
        execution_ref: Option<ResourceRef>,
        force: bool,
        create: bool,
        deadline: RequestDeadline,
    ) -> Result<(), CliFailure> {
        let target = ProcessAttachTarget::shell_session(
            self.zone_path.clone(),
            session_ref,
            execution_ref,
            force,
        )
        .map_err(|_| CliFailure::new(2, "ref-invalid: invalid ShellSession target"))?;
        let stream = self
            .backend
            .canonical
            .open_attach_stream(
                target,
                if create { "Create" } else { "Attach" },
                true,
                true,
                deadline,
            )
            .map_err(|error| self.client_failure(error, OutputMode::Human))?;
        let mut guard = crate::exec_client::FdStateGuard::enter(true, true)
            .map_err(|_| CliFailure::new(69, "shell terminal setup failed"))?;
        let mut host = crate::exec_client::RealHostIo;
        let mut signals = crate::exec_client::install_signals()
            .map_err(|_| CliFailure::new(69, "shell signal setup failed"))?;
        let mut input = [0_u8; d2b_contracts_control::public_wire::EXEC_MAX_CHUNK_BYTES as usize];
        loop {
            for signal in crate::terminal_client::TerminalSignalSource::drain(&mut signals) {
                match signal {
                    crate::exec_client::ExecSignal::Winch => {
                        if let Some((rows, cols)) = host.window_size() {
                            let size = u16::try_from(rows)
                                .ok()
                                .zip(u16::try_from(cols).ok())
                                .and_then(|(rows, cols)| TerminalSize::new(rows, cols).ok());
                            if let Some(size) = size {
                                let _ = block_on(stream.resize(size));
                            }
                        }
                    }
                    crate::exec_client::ExecSignal::Hangup
                    | crate::exec_client::ExecSignal::Terminate
                    | crate::exec_client::ExecSignal::Stop
                    | crate::exec_client::ExecSignal::Interrupt
                    | crate::exec_client::ExecSignal::Quit => {
                        let _ = block_on(stream.cancel());
                        guard.restore();
                        return Ok(());
                    }
                }
            }
            match host.read_stdin(&mut input) {
                Ok(0) => {
                    let _ = block_on(stream.close());
                    guard.restore();
                    return Ok(());
                }
                Ok(read) => {
                    block_on(stream.send(&input[..read]))
                        .map_err(|_| CliFailure::new(69, "shell input transport failed"))?;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => {
                    let _ = block_on(stream.cancel());
                    guard.restore();
                    return Err(CliFailure::new(69, "shell input failed"));
                }
            }
            let output = match block_on(stream.receive()) {
                Ok(output) => output,
                Err(ClientError::Cancelled) => {
                    guard.restore();
                    return Ok(());
                }
                Err(_) => return Err(CliFailure::new(69, "shell output transport failed")),
            };
            if !output.is_empty() {
                host.write_stdout(&output)
                    .map_err(|_| CliFailure::new(69, "shell output failed"))?;
            }
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn invoke_injected(
        &self,
        client: &dyn SessionClient,
        method: &str,
        payload: Value,
        deadline: RequestDeadline,
        mode: OutputMode,
        service: Option<ZoneServiceKind>,
        session_verb: Option<&str>,
    ) -> Result<Value, CliFailure> {
        let request =
            self.request_value_with_service(method, payload, mode, service, session_verb)?;
        let request = serde_json::to_vec(&request).map_err(|_| {
            self.failure(
                "internal-error",
                "failed to encode resource request",
                mode,
                1,
            )
        })?;
        let response = client
            .invoke(&request, deadline)
            .map_err(|error| self.transport_failure(error, mode))?;
        let value: Value = serde_json::from_slice(&response).map_err(|_| {
            self.failure(
                "exec-protocol-error",
                "Zone returned an invalid resource response",
                mode,
                1,
            )
        })?;
        let value = self.validate_response(value, mode)?;
        self.decorate_response(value)
    }

    fn request_value(
        &self,
        method: &str,
        payload: Value,
        mode: OutputMode,
    ) -> Result<Value, CliFailure> {
        self.request_value_with_service(method, payload, mode, None, None)
    }

    fn request_value_with_service(
        &self,
        method: &str,
        payload: Value,
        mode: OutputMode,
        service: Option<ZoneServiceKind>,
        session_verb: Option<&str>,
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
        if let Some(service) = service {
            request.insert(
                "service".to_owned(),
                Value::String(service.package().to_owned()),
            );
        }
        if let Some(session_verb) = session_verb {
            request.insert(
                "sessionVerb".to_owned(),
                Value::String(session_verb.to_owned()),
            );
        }
        Ok(Value::Object(request))
    }

    fn validate_response(&self, value: Value, mode: OutputMode) -> Result<Value, CliFailure> {
        if !value.is_object() {
            return Err(self.failure(
                "resource-schema-invalid",
                "Zone returned a non-object resource response",
                mode,
                1,
            ));
        }
        if matches!(
            value.get("type").and_then(Value::as_str),
            Some("error" | "helloRejected")
        ) {
            let class = value
                .pointer("/error/errorClass")
                .and_then(Value::as_str)
                .or_else(|| value.get("errorClass").and_then(Value::as_str))
                .or_else(|| value.pointer("/error/kind").and_then(Value::as_str))
                .or_else(|| value.get("kind").and_then(Value::as_str))
                .unwrap_or_else(|| {
                    if value.get("type").and_then(Value::as_str) == Some("helloRejected") {
                        "exec-auth-error"
                    } else {
                        "internal-error"
                    }
                });
            let class = stable_error_class(class);
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("Zone rejected the resource request");
            return Err(self.failure(
                class,
                &bounded_message(message),
                mode,
                error_exit_code(class),
            ));
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
        Ok(value)
    }

    fn decorate_response(&self, mut value: Value) -> Result<Value, CliFailure> {
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

    fn client_failure(&self, error: ClientError, mode: OutputMode) -> CliFailure {
        let admission_recovery = matches!(&error, ClientError::AmbiguousMutation);
        let (class, message, exit_code) = match error {
            ClientError::InvalidTarget => {
                ("resource-schema-invalid", "resource target was invalid", 2)
            }
            ClientError::InvalidService => {
                ("resource-schema-invalid", "resource service was invalid", 2)
            }
            ClientError::InvalidMethod => {
                ("resource-schema-invalid", "resource method was invalid", 2)
            }
            ClientError::InvalidMetadata => (
                "resource-schema-invalid",
                "resource metadata was invalid",
                2,
            ),
            ClientError::IdempotencyRequired => (
                "resource-schema-invalid",
                "resource idempotency was invalid",
                2,
            ),
            ClientError::RouteUnavailable | ClientError::SessionLost => {
                ("zone-unavailable", "Zone runtime is unavailable", 1)
            }
            ClientError::TransportPolicyMismatch => (
                "exec-auth-error",
                "Zone session route authentication was rejected",
                77,
            ),
            ClientError::DeadlineExpired => {
                ("deadline-exceeded", "Zone request exceeded its deadline", 1)
            }
            ClientError::Cancelled => ("operation-cancelled", "Zone request was cancelled", 3),
            ClientError::TransportFailed => {
                ("zone-unavailable", "Zone transport is unavailable", 1)
            }
            ClientError::AmbiguousMutation => (
                "resource-conflict",
                "resource mutation outcome was ambiguous",
                1,
            ),
            ClientError::ContractViolation => (
                "exec-protocol-error",
                "Zone returned an invalid resource response",
                1,
            ),
            ClientError::RetryLimitExceeded => (
                "zone-unavailable",
                "Zone request retry budget was exhausted",
                1,
            ),
            ClientError::Remote { kind, .. } => resource_error_surface(kind),
        };
        let mut failure = self.failure(class, message, mode, exit_code);
        failure.admission_recovery = admission_recovery;
        failure
    }

    #[cfg(test)]
    fn transport_failure(&self, error: TransportError, mode: OutputMode) -> CliFailure {
        let admission_recovery = false;
        let mut failure = match error {
            TransportError::Unavailable | TransportError::Io => {
                self.failure("zone-unavailable", "Zone runtime is unavailable", mode, 1)
            }
            TransportError::InvalidResponse => self.failure(
                "exec-protocol-error",
                "Zone returned an invalid resource response",
                mode,
                1,
            ),
            TransportError::OversizedResponse | TransportError::AncillaryData => self.failure(
                "resource-schema-invalid",
                "Zone response exceeded the bounded response size",
                mode,
                1,
            ),
            TransportError::DeadlineExceeded => self.failure(
                "deadline-exceeded",
                "Zone request exceeded its deadline",
                mode,
                1,
            ),
            TransportError::AuthRejected => self.failure(
                "exec-auth-error",
                "Zone session authentication was rejected",
                mode,
                77,
            ),
        };
        failure.admission_recovery = admission_recovery;
        failure
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
            let event = self.decorate_envelope(event);
            let mut rendered = serde_json::to_string(&event).map_err(|_| {
                self.failure("internal-error", "failed to render watch event", mode, 1)
            })?;
            rendered.push('\n');
            print_stdout(&rendered);
        }
        Ok(())
    }

    fn decorate_envelope(&self, mut value: Value) -> Value {
        if let Value::Object(object) = &mut value {
            object.entry("ok".to_owned()).or_insert(Value::Bool(true));
            object
                .entry("zoneRef".to_owned())
                .or_insert_with(|| Value::String(self.zone_ref()));
            object
                .entry("schemaVersion".to_owned())
                .or_insert_with(|| Value::Number(serde_json::Number::from(JSON_SCHEMA_VERSION)));
        }
        value
    }
}

fn canonical_backend(zone_name: &str, socket_path: &Path) -> Result<ContextBackend, CliFailure> {
    let zone_path =
        zone_path(zone_name).map_err(|_| CliFailure::new(2, "ref-invalid: invalid Zone name"))?;
    Ok(ContextBackend {
        canonical: CanonicalZoneBackend {
            zone_name: zone_name.to_owned(),
            zone_path,
            socket_path: socket_path.to_owned(),
        },
        #[cfg(test)]
        injected: None,
    })
}

impl CanonicalZoneBackend {
    fn open_attach_stream(
        &self,
        target: ProcessAttachTarget,
        operation: &str,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
    ) -> Result<d2b_resource_client::ProcessAttachStream<CliAttachStream>, ClientError> {
        let connector = CliZoneConnector::new(
            self.zone_name.clone(),
            self.zone_path.clone(),
            self.socket_path.clone(),
            ZoneServiceKind::Zone,
            operation.to_owned(),
            Some("attach".to_owned()),
            deadline.duration(),
        );
        let initial_size = if tty {
            let (rows, cols) =
                crate::exec_client::current_window_size().ok_or(ClientError::InvalidMetadata)?;
            Some(TerminalSize::new(
                u16::try_from(rows).map_err(|_| ClientError::InvalidMetadata)?,
                u16::try_from(cols).map_err(|_| ClientError::InvalidMetadata)?,
            )?)
        } else {
            None
        };
        let attach_options = ProcessAttachOptions::new(interactive, tty, initial_size)?;
        let owner = owner_for_zone(&self.zone_path);
        let resolver = RouteTable::new(vec![RouteRecord::new(owner, TransportKind::LocalUnix)]);
        let client = ProcessAttachClient::new(resolver, connector);
        let call_options = call_options(deadline, ResourceVerb::Get)?;
        let cancellation = CancellationToken::default();
        block_on(client.attach(
            target,
            attach_options,
            call_options,
            TransportSelection::exact(TransportKind::LocalUnix),
            &cancellation,
        ))
    }

    fn invoke_service(
        &self,
        operation: &str,
        payload: Value,
        deadline: RequestDeadline,
        service: ZoneServiceKind,
        session_verb: Option<&str>,
    ) -> Result<Value, ClientError> {
        self.invoke_with_verb(
            operation,
            payload,
            deadline,
            service,
            session_verb,
            resource_verb(operation, false),
        )
    }

    fn invoke_with_verb(
        &self,
        operation: &str,
        payload: Value,
        deadline: RequestDeadline,
        service: ZoneServiceKind,
        session_verb: Option<&str>,
        verb: ResourceVerb,
    ) -> Result<Value, ClientError> {
        let payload = serde_json::to_vec(&payload).map_err(|_| ClientError::ContractViolation)?;
        let payload =
            CanonicalJsonObject::parse(&payload).map_err(|_| ClientError::ContractViolation)?;
        let options = call_options(deadline, verb)?;
        let cancellation = CancellationToken::default();
        let request = ResourceCallOptions::new(payload, false, &cancellation);
        let owner = owner_for_zone(&self.zone_path);
        let resolver = RouteTable::new(vec![RouteRecord::new(
            owner.clone(),
            TransportKind::LocalUnix,
        )]);
        let connector = CliZoneConnector::new(
            self.zone_name.clone(),
            self.zone_path.clone(),
            self.socket_path.clone(),
            service,
            operation.to_owned(),
            session_verb.map(str::to_owned),
            deadline.duration(),
        );
        let client = ZoneClient::new(resolver, connector);
        let target = TargetInput::Service { owner, service };
        let selection = TransportSelection::exact(TransportKind::LocalUnix);
        let connection = block_on(client.connect(&target, service, selection))?;
        let response = block_on(client.call_connected(&connection, verb, options, request))?;
        serde_json::from_slice(&response.to_canonical_bytes())
            .map_err(|_| ClientError::ContractViolation)
    }

    #[cfg(test)]
    fn attach_process(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
        injected: Option<Arc<dyn SessionClient>>,
    ) -> Result<(), ClientError> {
        self.attach_process_inner(resource_ref, interactive, tty, deadline, injected)
    }

    #[cfg(not(test))]
    fn attach_process(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
    ) -> Result<(), ClientError> {
        self.attach_process_inner(resource_ref, interactive, tty, deadline)
    }

    #[cfg(test)]
    fn attach_process_inner(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
        injected: Option<Arc<dyn SessionClient>>,
    ) -> Result<(), ClientError> {
        let mut connector = CliZoneConnector::new(
            self.zone_name.clone(),
            self.zone_path.clone(),
            self.socket_path.clone(),
            ZoneServiceKind::Zone,
            "Attach".to_owned(),
            Some("attach".to_owned()),
            deadline.duration(),
        );
        connector.injected = injected;
        self.attach_process_with_connector(resource_ref, interactive, tty, deadline, connector)
    }

    #[cfg(not(test))]
    fn attach_process_inner(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
    ) -> Result<(), ClientError> {
        let connector = CliZoneConnector::new(
            self.zone_name.clone(),
            self.zone_path.clone(),
            self.socket_path.clone(),
            ZoneServiceKind::Zone,
            "Attach".to_owned(),
            Some("attach".to_owned()),
            deadline.duration(),
        );
        self.attach_process_with_connector(resource_ref, interactive, tty, deadline, connector)
    }

    fn attach_process_with_connector(
        &self,
        resource_ref: ResourceRef,
        interactive: bool,
        tty: bool,
        deadline: RequestDeadline,
        connector: CliZoneConnector,
    ) -> Result<(), ClientError> {
        let target = ProcessAttachTarget::ephemeral_process(self.zone_path.clone(), resource_ref)?;
        let initial_size = if tty {
            let (rows, cols) =
                crate::exec_client::current_window_size().ok_or(ClientError::InvalidMetadata)?;
            Some(TerminalSize::new(
                u16::try_from(rows).map_err(|_| ClientError::InvalidMetadata)?,
                u16::try_from(cols).map_err(|_| ClientError::InvalidMetadata)?,
            )?)
        } else {
            None
        };
        let attach_options = ProcessAttachOptions::new(interactive, tty, initial_size)?;
        let owner = owner_for_zone(&self.zone_path);
        let resolver = RouteTable::new(vec![RouteRecord::new(owner, TransportKind::LocalUnix)]);
        let client = ProcessAttachClient::new(resolver, connector);
        let call_options = call_options(deadline, ResourceVerb::Get)?;
        let cancellation = CancellationToken::default();
        block_on(client.attach_and_close(
            target,
            attach_options,
            call_options,
            TransportSelection::exact(TransportKind::LocalUnix),
            &cancellation,
        ))
    }
}

/// The CLI's private bridge from the local authenticated session endpoint to
/// the transport-neutral resource client.
#[derive(Clone)]
struct CliZoneConnector {
    zone_name: String,
    zone_path: d2b_contracts_zone_session::v3::zone_routing::ZonePath,
    socket_path: PathBuf,
    service: ZoneServiceKind,
    operation: String,
    session_verb: Option<String>,
    handshake_timeout: Duration,
    #[cfg(test)]
    injected: Option<Arc<dyn SessionClient>>,
}

impl std::fmt::Debug for CliZoneConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CliZoneConnector")
            .field("zone_name", &self.zone_name)
            .field("service", &self.service)
            .field("operation", &self.operation)
            .field("session", &"<authenticated>")
            .finish()
    }
}

impl CliZoneConnector {
    fn new(
        zone_name: String,
        zone_path: d2b_contracts_zone_session::v3::zone_routing::ZonePath,
        socket_path: PathBuf,
        service: ZoneServiceKind,
        operation: String,
        session_verb: Option<String>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            zone_name,
            zone_path,
            socket_path,
            service,
            operation,
            session_verb,
            handshake_timeout: request_timeout
                .min(Duration::from_millis(LOCAL_HANDSHAKE_DEADLINE_MS)),
            #[cfg(test)]
            injected: None,
        }
    }

    fn connect_now(
        &self,
        target: &d2b_resource_client::ResolvedTarget,
        service: ZoneServiceKind,
    ) -> Result<(CliConnectedSession, ZoneSessionPin), ClientError> {
        if !matches!(
            service,
            ZoneServiceKind::Resource
                | ZoneServiceKind::Zone
                | ZoneServiceKind::Audit
                | ZoneServiceKind::Support
                | ZoneServiceKind::ConfigNixos
        ) || target.service() != service
            || target.transport() != TransportKind::LocalUnix
            || target.owner().zone() != &self.zone_path
        {
            return Err(ClientError::TransportPolicyMismatch);
        }
        let operation = validate_operation(service, &self.operation, self.session_verb.as_deref())?;
        #[cfg(test)]
        if self.injected.is_some() {
            let peer = ZonePeerIdentity::from_observed_static_key(
                self.zone_path.clone(),
                peer_fingerprint(&self.zone_name),
            );
            let pin = ZoneSessionPin::new(peer, service, TransportKind::LocalUnix, 1, [0xA5; 32])?;
            return Ok((
                CliConnectedSession {
                    zone_name: self.zone_name.clone(),
                    service,
                    operation,
                    session_verb: self.session_verb.clone(),
                    socket: None,
                    #[cfg(test)]
                    injected: self.injected.clone(),
                },
                pin,
            ));
        }
        let mut socket =
            SeqpacketUnixSocket::connect(&self.socket_path).map_err(classify_client_io_error)?;
        socket
            .set_io_timeout(self.handshake_timeout)
            .map_err(classify_client_io_error)?;
        let hello =
            daemon_hello_frame("hello").map_err(|_| ClientError::ContractViolation)?;
        socket
            .send_frame(&hello)
            .map_err(classify_client_io_error)?;
        let hello_reply = socket.recv_frame().map_err(classify_client_io_error)?;
        let hello_type = serde_json::from_slice::<Value>(&hello_reply)
            .ok()
            .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned));
        if hello_type.as_deref() != Some("helloOk") {
            return Err(if hello_type.as_deref() == Some("helloRejected") {
                ClientError::Remote {
                    kind: ResourceErrorKind::AuthorizationDenied,
                    retry: RetryClass::Never,
                }
            } else {
                ClientError::ContractViolation
            });
        }
        let peer = ZonePeerIdentity::from_observed_static_key(
            self.zone_path.clone(),
            peer_fingerprint(&self.zone_name),
        );
        let transcript_hash: [u8; 32] = Sha256::digest(&hello_reply).into();
        let pin = ZoneSessionPin::new(
            peer.clone(),
            service,
            TransportKind::LocalUnix,
            1,
            transcript_hash,
        )?;
        ZoneSocketConnector::new(peer).verify_session_pin(&pin)?;
        Ok((
            CliConnectedSession {
                zone_name: self.zone_name.clone(),
                service,
                operation,
                session_verb: self.session_verb.clone(),
                socket: Some(Arc::new(Mutex::new(socket))),
                #[cfg(test)]
                injected: self.injected.clone(),
            },
            pin,
        ))
    }
}

impl ZoneSessionConnector for CliZoneConnector {
    type Session = CliConnectedSession;

    fn connect(
        &self,
        target: &d2b_resource_client::ResolvedTarget,
        service: ZoneServiceKind,
    ) -> impl Future<Output = Result<(Self::Session, ZoneSessionPin), ClientError>> + Send {
        ready(self.connect_now(target, service))
    }
}

#[derive(Clone)]
struct CliConnectedSession {
    zone_name: String,
    service: ZoneServiceKind,
    operation: String,
    session_verb: Option<String>,
    socket: Option<Arc<Mutex<SeqpacketUnixSocket>>>,
    #[cfg(test)]
    injected: Option<Arc<dyn SessionClient>>,
}

impl std::fmt::Debug for CliConnectedSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CliConnectedSession(<authenticated>)")
    }
}

/// Local transport adapter for an authenticated process attachment stream.
///
/// Process attachments need only establishment, while ShellSession attachments
/// retain the socket for bounded stdin, output, resize, cancellation, and
/// close messages on the admitted named stream.
struct CliAttachStream {
    closed: AtomicBool,
    teardown_sent: AtomicBool,
    eof: AtomicBool,
    socket: Option<Arc<Mutex<SeqpacketUnixSocket>>>,
    stdin_offset: Mutex<u64>,
    stdout_offset: AtomicU64,
    next_request_id: AtomicU64,
    control_sequence: AtomicU64,
}

impl CliAttachStream {
    fn new(socket: Option<Arc<Mutex<SeqpacketUnixSocket>>>) -> Self {
        Self {
            closed: AtomicBool::new(false),
            teardown_sent: AtomicBool::new(false),
            eof: AtomicBool::new(false),
            socket,
            stdin_offset: Mutex::new(0),
            stdout_offset: AtomicU64::new(0),
            next_request_id: AtomicU64::new(1),
            control_sequence: AtomicU64::new(1),
        }
    }
}

impl NamedStreamTransport for CliAttachStream {
    fn send(&self, bytes: Vec<u8>) -> impl Future<Output = Result<(), ClientError>> + Send {
        ready(self.send_stdin(&bytes))
    }

    fn resize(&self, size: TerminalSize) -> impl Future<Output = Result<(), ClientError>> + Send {
        ready(
            self.stream_round_trip(NamedProcessStreamRequest::Resize {
                control_seq: self.control_sequence.fetch_add(1, Ordering::AcqRel),
                rows: u32::from(size.rows()),
                cols: u32::from(size.cols()),
            })
            .and_then(|response| match response {
                NamedProcessStreamResponse::Delivered(_) => Ok(()),
                _ => Err(ClientError::ContractViolation),
            }),
        )
    }

    fn receive(&self) -> impl Future<Output = Result<Vec<u8>, ClientError>> + Send {
        if self.eof.load(Ordering::Acquire) {
            return ready(Err(ClientError::Cancelled));
        }
        let result = self
            .stream_round_trip(NamedProcessStreamRequest::Read {
                stream: ExecStream::Stdout,
                offset: self.stdout_offset.load(Ordering::Acquire),
                max_len: d2b_contracts_control::public_wire::EXEC_MAX_CHUNK_BYTES,
                wait: true,
                timeout_ms: 50,
            })
            .and_then(|response| match response {
                NamedProcessStreamResponse::Output(ExecReadOutputResult {
                    data_base64,
                    next_offset,
                    eof,
                    ..
                }) => {
                    let data = d2b_core::base64_codec::decode(&data_base64)
                        .map_err(|_| ClientError::ContractViolation)?;
                    self.stdout_offset.store(next_offset, Ordering::Release);
                    if eof {
                        self.eof.store(true, Ordering::Release);
                        if data.is_empty() {
                            return Err(ClientError::Cancelled);
                        }
                    }
                    Ok(data)
                }
                NamedProcessStreamResponse::Terminal(_) => {
                    self.eof.store(true, Ordering::Release);
                    Err(ClientError::Cancelled)
                }
                _ => Err(ClientError::ContractViolation),
            });
        ready(result)
    }

    fn close(&self) -> impl Future<Output = Result<(), ClientError>> + Send {
        self.closed.store(true, Ordering::Release);
        let result = if self.socket.is_some() {
            self.stream_round_trip(NamedProcessStreamRequest::Close)
                .and_then(|response| match response {
                    NamedProcessStreamResponse::Closed(_) => Ok(()),
                    _ => Err(ClientError::ContractViolation),
                })
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.teardown_sent.store(true, Ordering::Release);
        }
        ready(result)
    }

    fn cancel(&self) -> impl Future<Output = Result<(), ClientError>> + Send {
        self.closed.store(true, Ordering::Release);
        let result = if self.socket.is_some() {
            self.stream_round_trip(NamedProcessStreamRequest::Cancel)
                .and_then(|response| match response {
                    NamedProcessStreamResponse::Closed(_)
                    | NamedProcessStreamResponse::Delivered(_) => Ok(()),
                    _ => Err(ClientError::ContractViolation),
                })
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.teardown_sent.store(true, Ordering::Release);
        }
        ready(result)
    }
}

impl Drop for CliAttachStream {
    fn drop(&mut self) {
        if self.teardown_sent.swap(true, Ordering::AcqRel) || self.socket.is_none() {
            return;
        }
        let _ = self.stream_round_trip(NamedProcessStreamRequest::Cancel);
    }
}

impl CliAttachStream {
    fn send_stdin(&self, bytes: &[u8]) -> Result<(), ClientError> {
        let mut offset = self
            .stdin_offset
            .lock()
            .map_err(|_| ClientError::SessionLost)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut consumed = 0;
        while consumed < bytes.len() {
            let response = self.stream_round_trip(NamedProcessStreamRequest::Stdin {
                offset: *offset,
                chunk_base64: d2b_core::base64_codec::encode(&bytes[consumed..]),
                eof: false,
            })?;
            let NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                accepted_len,
                next_offset,
                backpressured,
                stdin_closed,
            }) = response
            else {
                return Err(ClientError::ContractViolation);
            };
            let accepted =
                usize::try_from(accepted_len).map_err(|_| ClientError::ContractViolation)?;
            if accepted > bytes.len() - consumed
                || next_offset != (*offset).saturating_add(accepted as u64)
            {
                return Err(ClientError::ContractViolation);
            }
            if accepted == 0 {
                if stdin_closed {
                    return Err(ClientError::SessionLost);
                }
                if !backpressured {
                    return Err(ClientError::ContractViolation);
                }
                if Instant::now() >= deadline {
                    return Err(ClientError::Remote {
                        kind: ResourceErrorKind::Backpressure,
                        retry: RetryClass::AfterDelay,
                    });
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            *offset = next_offset;
            consumed += accepted;
        }
        Ok(())
    }

    fn stream_round_trip(
        &self,
        request: NamedProcessStreamRequest,
    ) -> Result<NamedProcessStreamResponse, ClientError> {
        let socket = self.socket.as_ref().ok_or(ClientError::ContractViolation)?;
        let request_id = self.next_request_id.fetch_add(1, Ordering::AcqRel);
        if request_id == 0 {
            return Err(ClientError::ContractViolation);
        }
        let frame = NamedProcessStreamRequestFrame::new(request_id, request);
        let bytes = serde_json::to_vec(&frame).map_err(|_| ClientError::ContractViolation)?;
        let mut socket = socket.lock().map_err(|_| ClientError::SessionLost)?;
        socket
            .send_frame(&bytes)
            .map_err(|_| ClientError::TransportFailed)?;
        let response = socket.recv_frame().map_err(|_| ClientError::SessionLost)?;
        let frame: NamedProcessStreamResponseFrame =
            serde_json::from_slice(&response).map_err(|_| ClientError::ContractViolation)?;
        if frame.request_id != request_id {
            return Err(ClientError::ContractViolation);
        }
        if let NamedProcessStreamResponse::Error(error) = frame.response {
            return Err(named_stream_client_error(error.kind));
        }
        Ok(frame.response)
    }
}

fn named_stream_client_error(kind: NamedProcessStreamErrorKind) -> ClientError {
    let (kind, retry) = match kind {
        NamedProcessStreamErrorKind::Authorization => (
            ResourceErrorKind::AuthorizationDenied,
            RetryClass::Reauthorize,
        ),
        NamedProcessStreamErrorKind::StaleSession | NamedProcessStreamErrorKind::NotFound => {
            (ResourceErrorKind::ResourceNotFound, RetryClass::Never)
        }
        NamedProcessStreamErrorKind::Backpressure => {
            (ResourceErrorKind::Backpressure, RetryClass::AfterDelay)
        }
        NamedProcessStreamErrorKind::Protocol => {
            (ResourceErrorKind::ResourceSchemaInvalid, RetryClass::Never)
        }
        NamedProcessStreamErrorKind::Timeout => {
            (ResourceErrorKind::Timeout, RetryClass::AfterDelay)
        }
        NamedProcessStreamErrorKind::Disconnected => (
            ResourceErrorKind::ResourceProviderUnavailable,
            RetryClass::AfterDelay,
        ),
    };
    ClientError::Remote { kind, retry }
}

impl ConnectedZoneSession for CliConnectedSession {
    fn call(
        &self,
        verb: ResourceVerb,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
        self.call_with_timeout(verb, target, payload, u64::MAX)
    }

    fn call_with_timeout(
        &self,
        _verb: ResourceVerb,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
        relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
        let result = self.invoke(target, payload, relative_timeout_nanos);
        ready(result)
    }

    fn call_scoped_commit_batch(
        &self,
        _assignment: AssignmentIdentity,
        _mutations: Vec<ScopedResourceMutation>,
        _payload: CanonicalJsonObject,
        _relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
        // The CLI public socket is an operator route, not a controller
        // ComponentSession. Never downgrade a scoped write to plain CommitBatch.
        ready(Err(ClientError::ContractViolation))
    }
}

impl ConnectedSession for CliConnectedSession {
    type Stream = CliAttachStream;

    fn open_named_stream(
        &self,
        request: ProcessAttachOpenRequest,
        relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<Self::Stream, ClientError>> + Send {
        let result = self.open_process_attach(request, relative_timeout_nanos);
        ready(result)
    }
}

impl CliConnectedSession {
    fn open_process_attach(
        &self,
        request: ProcessAttachOpenRequest,
        relative_timeout_nanos: u64,
    ) -> Result<CliAttachStream, ClientError> {
        if self.service != ZoneServiceKind::Zone
            || !matches!(self.operation.as_str(), "Attach" | "Create")
            || self.session_verb.as_deref() != Some("attach")
        {
            return Err(ClientError::TransportPolicyMismatch);
        }
        let options = request.options();
        let initial_size = options.initial_size().map(|size| {
            json!({
                "cols": size.cols(),
                "rows": size.rows(),
            })
        });
        let mut request_payload = json!({
            "interactive": options.interactive(),
            "initialSize": initial_size,
            "tty": options.tty(),
        });
        if let ProcessAttachTarget::ShellSession {
            execution_ref,
            force,
            ..
        } = request.target()
            && let Some(object) = request_payload.as_object_mut()
        {
            object.insert(
                "executionRef".to_owned(),
                execution_ref
                    .as_ref()
                    .map(|reference| Value::String(reference.to_canonical_string()))
                    .unwrap_or(Value::Null),
            );
            object.insert("force".to_owned(), Value::Bool(*force));
        }
        let payload =
            serde_json::to_vec(&request_payload).map_err(|_| ClientError::ContractViolation)?;
        let payload =
            CanonicalJsonObject::parse(&payload).map_err(|_| ClientError::ContractViolation)?;
        self.invoke(
            Some(request.target().resource_ref().clone()),
            payload,
            relative_timeout_nanos,
        )?;
        let socket = self.socket.clone();
        let is_shell = matches!(request.target(), ProcessAttachTarget::ShellSession { .. });
        Ok(CliAttachStream::new(is_shell.then_some(socket).flatten()))
    }

    fn invoke(
        &self,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
        relative_timeout_nanos: u64,
    ) -> Result<CanonicalJsonObject, ClientError> {
        let mut request: Value = serde_json::from_slice(&payload.to_canonical_bytes())
            .map_err(|_| ClientError::ContractViolation)?;
        let object = request
            .as_object_mut()
            .ok_or(ClientError::ContractViolation)?;
        object.insert(
            "type".to_owned(),
            Value::String("resourceRequest".to_owned()),
        );
        object.insert("method".to_owned(), Value::String(self.operation.clone()));
        object.insert(
            "service".to_owned(),
            Value::String(self.service.package().to_owned()),
        );
        object.insert(
            "zoneRef".to_owned(),
            Value::String(format!("Zone/{}", self.zone_name)),
        );
        object.insert(
            "schemaVersion".to_owned(),
            Value::Number(serde_json::Number::from(JSON_SCHEMA_VERSION)),
        );
        if let Some(session_verb) = &self.session_verb {
            object.insert(
                "sessionVerb".to_owned(),
                Value::String(session_verb.clone()),
            );
        }
        if let Some(target) = target {
            object
                .entry("resourceRef".to_owned())
                .or_insert_with(|| Value::String(target.to_canonical_string()));
        }
        let request = serde_json::to_vec(&request).map_err(|_| ClientError::ContractViolation)?;
        let timeout = Duration::from_nanos(relative_timeout_nanos.max(1));
        #[cfg(test)]
        if let Some(client) = &self.injected {
            let response = client
                .invoke(&request, RequestDeadline(timeout))
                .map_err(|error| match error {
                    TransportError::Unavailable | TransportError::Io => ClientError::SessionLost,
                    TransportError::DeadlineExceeded => ClientError::DeadlineExpired,
                    TransportError::InvalidResponse
                    | TransportError::OversizedResponse
                    | TransportError::AncillaryData => ClientError::ContractViolation,
                    TransportError::AuthRejected => ClientError::Remote {
                        kind: ResourceErrorKind::AuthorizationDenied,
                        retry: RetryClass::Never,
                    },
                })?;
            return decode_cli_response(&response);
        }
        let mut socket = self
            .socket
            .as_ref()
            .ok_or(ClientError::TransportFailed)?
            .lock()
            .map_err(|_| ClientError::TransportFailed)?;
        socket
            .set_io_timeout(timeout)
            .map_err(classify_client_io_error)?;
        socket
            .send_frame(&request)
            .map_err(classify_client_io_error)?;
        let response = socket.recv_frame().map_err(classify_client_io_error)?;
        decode_cli_response(&response)
    }
}

fn decode_cli_response(response: &[u8]) -> Result<CanonicalJsonObject, ClientError> {
    if response.len() > MAX_FRAME_BYTES {
        return Err(ClientError::ContractViolation);
    }
    let value: Value =
        serde_json::from_slice(response).map_err(|_| ClientError::ContractViolation)?;
    if !value.is_object() {
        return Err(ClientError::ContractViolation);
    }
    if matches!(
        value.get("type").and_then(Value::as_str),
        Some("error" | "helloRejected")
    ) || value
        .get("ok")
        .and_then(Value::as_bool)
        .is_some_and(|ok| !ok)
    {
        return Err(remote_client_error(&value));
    }
    CanonicalJsonObject::parse(response).map_err(|_| ClientError::ContractViolation)
}

fn zone_path(
    zone_name: &str,
) -> Result<d2b_contracts_zone_session::v3::zone_routing::ZonePath, ()> {
    let label =
        d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId::parse(zone_name.to_owned())
            .map_err(|_| ())?;
    d2b_contracts_zone_session::v3::zone_routing::ZonePath::new(vec![label]).map_err(|_| ())
}

fn owner_for_zone(
    zone_path: &d2b_contracts_zone_session::v3::zone_routing::ZonePath,
) -> ServiceOwner {
    if zone_path == &d2b_contracts_zone_session::v3::zone_routing::ZonePath::local_root() {
        ServiceOwner::ZoneLocal(zone_path.clone())
    } else {
        ServiceOwner::Zone(zone_path.clone())
    }
}

fn validate_operation(
    service: ZoneServiceKind,
    operation: &str,
    session_verb: Option<&str>,
) -> Result<String, ClientError> {
    match (service, operation, session_verb) {
        (ZoneServiceKind::Audit, "AuditService/Export", Some("audit-export"))
        | (ZoneServiceKind::Support, "SupportService/GenerateBundle", Some("support-bundle")) => {
            Ok(operation.to_owned())
        }
        (
            ZoneServiceKind::ConfigNixos,
            "ConfigNixosService/ReadGuestConfig"
                | "ConfigNixosService/Stage"
                | "ConfigNixosService/Diff"
                | "ConfigNixosService/Approve"
                | "ConfigNixosService/Reject"
                | "ConfigNixosService/Status",
            Some("invoke"),
        ) => Ok(operation.to_owned()),
        (ZoneServiceKind::Zone, "Attach" | "Create", Some("attach")) => Ok(operation.to_owned()),
        (ZoneServiceKind::Audit | ZoneServiceKind::Support, ..) => Err(ClientError::InvalidMethod),
        (_, _, Some(_)) => Err(ClientError::InvalidMethod),
        (_, operation, None)
            if !operation.is_empty()
                && operation.len() <= 64
                && operation
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_') =>
        {
            Ok(operation.to_owned())
        }
        _ => Err(ClientError::InvalidMethod),
    }
}

fn canonical_verb(method: &str) -> ResourceVerb {
    match method {
        "List" => ResourceVerb::List,
        "Watch" => ResourceVerb::Watch,
        "Create" | "DeviceUsbAttach" | "DeviceUsbDetach" | "SecurityKeyCancel" | "Apply" => {
            ResourceVerb::Create
        }
        "UpdateSpec" | "Start" | "Stop" | "Restart" => ResourceVerb::UpdateSpec,
        "Delete" => ResourceVerb::Delete,
        "Upgrade" => ResourceVerb::Upgrade,
        _ => ResourceVerb::Get,
    }
}

fn resource_verb(method: &str, mutating: bool) -> ResourceVerb {
    if mutating && matches!(method, "Start" | "Stop" | "Restart") {
        ResourceVerb::UpdateSpec
    } else if mutating {
        ResourceVerb::Create
    } else {
        canonical_verb(method)
    }
}

fn operation_service(method: &str) -> ZoneServiceKind {
    match method {
        "ZoneGet" | "ZoneList" | "ZoneStatus" => ZoneServiceKind::Zone,
        "AuditService/Export" => ZoneServiceKind::Audit,
        "SupportService/GenerateBundle" => ZoneServiceKind::Support,
        _ => ZoneServiceKind::Resource,
    }
}

fn call_options(deadline: RequestDeadline, verb: ResourceVerb) -> Result<CallOptions, ClientError> {
    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    let issued = SystemClock.now_unix_ms().max(1);
    let lifetime_ms =
        u64::try_from(deadline.duration().as_millis()).map_err(|_| ClientError::InvalidMetadata)?;
    let expires = issued
        .checked_add(lifetime_ms)
        .ok_or(ClientError::InvalidMetadata)?;
    let sequence = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let mut request_id = [0_u8; d2b_resource_client::REQUEST_ID_BYTES];
    request_id[..8].copy_from_slice(&issued.to_le_bytes());
    request_id[8..].copy_from_slice(&sequence.to_le_bytes());
    let mut metadata = MetadataInput::new(request_id, issued, expires)?;
    if resource_verb_is_mutating(verb) {
        metadata = metadata.with_idempotency(request_id.to_vec())?;
    }
    Ok(CallOptions {
        metadata,
        retry: RetryPolicy::no_retry(),
    })
}

fn peer_fingerprint(zone_name: &str) -> [u8; 32] {
    Sha256::digest(format!("d2b-cli-zone-peer-v3:{zone_name}").as_bytes()).into()
}

fn classify_client_io_error(error: io::Error) -> ClientError {
    match error.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => ClientError::SessionLost,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ClientError::DeadlineExpired,
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => ClientError::ContractViolation,
        _ => ClientError::TransportFailed,
    }
}

fn remote_client_error(value: &Value) -> ClientError {
    let class = value
        .pointer("/error/errorClass")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/kind").and_then(Value::as_str))
        .or_else(|| value.get("errorClass").and_then(Value::as_str))
        .or_else(|| value.get("kind").and_then(Value::as_str))
        .unwrap_or("internal-integrity-failure");
    let kind = resource_error_kind(class);
    let retry = value
        .pointer("/error/retryClass")
        .and_then(Value::as_str)
        .or_else(|| value.get("retryClass").and_then(Value::as_str))
        .map(retry_class)
        .unwrap_or(RetryClass::Never);
    ClientError::Remote { kind, retry }
}

fn resource_error_kind(value: &str) -> ResourceErrorKind {
    match value {
        "resource-not-found" => ResourceErrorKind::ResourceNotFound,
        "resource-already-exists" => ResourceErrorKind::ResourceAlreadyExists,
        "resource-conflict" => ResourceErrorKind::ResourceConflict,
        "resource-schema-invalid" | "wire-invalid-frame" => {
            ResourceErrorKind::ResourceSchemaInvalid
        }
        "resource-ref-invalid" | "ref-invalid" => ResourceErrorKind::ResourceRefInvalid,
        "resource-owner-cycle" => ResourceErrorKind::ResourceOwnerCycle,
        "resource-owner-depth" => ResourceErrorKind::ResourceOwnerDepth,
        "resource-finalizer-denied" => ResourceErrorKind::ResourceFinalizerDenied,
        "resource-provider-unavailable" | "provider-unavailable" => {
            ResourceErrorKind::ResourceProviderUnavailable
        }
        "resource-controller-mismatch" => ResourceErrorKind::ResourceControllerMismatch,
        "resource-status-owner-mismatch" => ResourceErrorKind::ResourceStatusOwnerMismatch,
        "status-oversize" => ResourceErrorKind::StatusOversize,
        "status-provider-schema-invalid" => ResourceErrorKind::StatusProviderSchemaInvalid,
        "status-provider-overlap" => ResourceErrorKind::StatusProviderOverlap,
        "spec-provider-schema-invalid" => ResourceErrorKind::SpecProviderSchemaInvalid,
        "spec-provider-shadow" => ResourceErrorKind::SpecProviderShadow,
        "unsupported-capability" => ResourceErrorKind::UnsupportedCapability,
        "expedited-not-authorized" => ResourceErrorKind::ExpeditedNotAuthorized,
        "expedited-quota-exceeded" => ResourceErrorKind::ExpeditedQuotaExceeded,
        "expedited-reconcile-pending" => ResourceErrorKind::ExpeditedReconcilePending,
        "upgrade-required" => ResourceErrorKind::UpgradeRequired,
        "endpoint-resolve-denied" => ResourceErrorKind::EndpointResolveDenied,
        "relay-denied" => ResourceErrorKind::RelayDenied,
        "role-relay-grant-restricted" => ResourceErrorKind::RoleRelayGrantRestricted,
        "authorization-denied" | "exec-auth-error" => ResourceErrorKind::AuthorizationDenied,
        "revision-expired" => ResourceErrorKind::RevisionExpired,
        "backpressure" => ResourceErrorKind::Backpressure,
        "timeout" | "deadline-exceeded" => ResourceErrorKind::Timeout,
        "cancelled" | "operation-cancelled" => ResourceErrorKind::Cancelled,
        "zone-unavailable" | "resource-plane-unavailable" => {
            ResourceErrorKind::ResourcePlaneUnavailable
        }
        _ => ResourceErrorKind::InternalIntegrityFailure,
    }
}

fn retry_class(value: &str) -> RetryClass {
    match value {
        "immediate" => RetryClass::Immediate,
        "after-delay" => RetryClass::AfterDelay,
        "reauthorize" => RetryClass::Reauthorize,
        _ => RetryClass::Never,
    }
}

fn resource_error_surface(kind: ResourceErrorKind) -> (&'static str, &'static str, i32) {
    match kind {
        ResourceErrorKind::ResourceNotFound => ("resource-not-found", "resource was not found", 1),
        ResourceErrorKind::ResourceAlreadyExists => {
            ("resource-already-exists", "resource already exists", 1)
        }
        ResourceErrorKind::ResourceConflict | ResourceErrorKind::RevisionExpired => {
            ("resource-conflict", "resource revision conflict", 1)
        }
        ResourceErrorKind::ResourceSchemaInvalid
        | ResourceErrorKind::ResourceRefInvalid
        | ResourceErrorKind::ResourceOwnerCycle
        | ResourceErrorKind::ResourceOwnerDepth
        | ResourceErrorKind::StatusOversize
        | ResourceErrorKind::StatusProviderSchemaInvalid
        | ResourceErrorKind::StatusProviderOverlap
        | ResourceErrorKind::SpecProviderSchemaInvalid
        | ResourceErrorKind::SpecProviderShadow => (
            "resource-schema-invalid",
            "Zone rejected the resource schema",
            2,
        ),
        ResourceErrorKind::ResourceProviderUnavailable => (
            "provider-unavailable",
            "resource Provider is unavailable",
            1,
        ),
        ResourceErrorKind::AuthorizationDenied
        | ResourceErrorKind::EndpointResolveDenied
        | ResourceErrorKind::RelayDenied
        | ResourceErrorKind::RoleRelayGrantRestricted
        | ResourceErrorKind::ExpeditedNotAuthorized => (
            "authorization-denied",
            "resource request was not authorized",
            1,
        ),
        ResourceErrorKind::Timeout => {
            ("deadline-exceeded", "Zone request exceeded its deadline", 1)
        }
        ResourceErrorKind::Cancelled => ("operation-cancelled", "Zone request was cancelled", 3),
        ResourceErrorKind::ResourcePlaneUnavailable | ResourceErrorKind::Backpressure => {
            ("zone-unavailable", "Zone runtime is unavailable", 1)
        }
        _ => ("internal-error", "Zone rejected the resource request", 1),
    }
}

struct ThreadWaker(thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let current = thread::current();
    let waker = Waker::from(Arc::new(ThreadWaker(current.clone())));
    let mut future = Box::pin(future);
    let mut context = TaskContext::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
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
        read_bounded_file(path)?
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

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, CliFailure> {
    let file =
        fs::File::open(path).map_err(|_| CliFailure::new(1, "failed to read resource spec"))?;
    let mut bytes = Vec::new();
    file.take((MAX_SPEC_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CliFailure::new(1, "failed to read resource spec"))?;
    if bytes.len() > MAX_SPEC_BYTES {
        return Err(CliFailure::new(2, "resource spec exceeds the 64 KiB bound"));
    }
    Ok(bytes)
}

pub(crate) fn bounded_message(message: &str) -> String {
    let mut bounded = String::new();
    for character in message
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    {
        if bounded.len() + character.len_utf8() > 4096 {
            break;
        }
        bounded.push(character);
    }
    bounded
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

#[cfg(test)]
fn classify_transport_error(error: &io::Error) -> TransportError {
    match error.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => TransportError::Unavailable,
        io::ErrorKind::InvalidData if error.to_string().contains("ancillary") => {
            TransportError::AncillaryData
        }
        io::ErrorKind::InvalidData if error.to_string().contains("oversized") => {
            TransportError::OversizedResponse
        }
        io::ErrorKind::InvalidData => TransportError::InvalidResponse,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TransportError::DeadlineExceeded,
        _ => TransportError::Io,
    }
}

fn socket_reachable(path: &Path) -> bool {
    let Ok(mut socket) = SeqpacketUnixSocket::connect(path) else {
        return false;
    };
    let timeout = Duration::from_millis(LOCAL_HANDSHAKE_DEADLINE_MS);
    if socket.set_io_timeout(timeout).is_err() {
        return false;
    }
    let Ok(hello) = daemon_hello_frame("hello") else {
        return false;
    };
    if socket.send_frame(&hello).is_err() {
        return false;
    }
    let Ok(reply) = socket.recv_frame() else {
        return false;
    };
    serde_json::from_slice::<Value>(&reply)
        .ok()
        .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        == Some("helloOk")
}

fn error_exit_code(class: &str) -> i32 {
    match class {
        "ref-invalid" | "resource-schema-invalid" => 2,
        "operation-cancelled" => 3,
        "exec-internal-error" => 42,
        "exec-transport-error" => 69,
        "exec-old-generation" => 70,
        "exec-capacity" => 75,
        "exec-protocol-error" => 76,
        "exec-auth-error" => 77,
        "not-implemented" => 78,
        _ => 1,
    }
}

fn stable_error_class(class: &str) -> &str {
    match class {
        "resource-not-found"
        | "resource-already-exists"
        | "resource-conflict"
        | "resource-schema-invalid"
        | "ref-invalid"
        | "authorization-denied"
        | "zone-unavailable"
        | "deadline-exceeded"
        | "operation-cancelled"
        | "provider-unavailable"
        | "exec-transport-error"
        | "exec-old-generation"
        | "exec-capacity"
        | "exec-protocol-error"
        | "exec-auth-error"
        | "exec-internal-error"
        | "shell-transport-error"
        | "not-implemented"
        | "internal-error"
        | "bundle-integrity-failure"
        | "bundle-generation-replay"
        | "bundle-schema-mismatch"
        | "resource-pending-cleanup" => class,
        _ => "internal-error",
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
                .get("status")
                .and_then(Value::as_object)
                .and_then(|status| status.get("isolationPosture"))
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
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockClient {
        requests: Mutex<Vec<Vec<u8>>>,
        response: Vec<u8>,
    }

    #[cfg(test)]
    mod transport_contract_tests {
        use super::{MAX_FRAME_BYTES, SeqpacketUnixSocket};
        use nix::sys::socket::{AddressFamily, MsgFlags, SockFlag, SockType, send, socketpair};
        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
        use std::{
            io::IoSlice,
            os::fd::{AsFd as _, AsRawFd as _},
        };

        #[test]
        fn seqpacket_client_rejects_oversized_declared_packets() {
            let (client, server) = socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC,
            )
            .expect("create seqpacket pair");
            let mut socket = SeqpacketUnixSocket { fd: client };
            let outbound = socket
                .send_frame(&vec![0_u8; MAX_FRAME_BYTES + 1])
                .expect_err("outbound oversized frame must fail closed");
            assert_eq!(outbound.kind(), std::io::ErrorKind::InvalidInput);
            let payload_len = MAX_FRAME_BYTES + 1;
            let mut frame = Vec::with_capacity(4);
            frame.extend_from_slice(&(payload_len as u32).to_le_bytes());
            send(server.as_raw_fd(), &frame, MsgFlags::empty()).expect("send oversized declaration");
            let error = socket
                .recv_frame()
                .expect_err("oversized declaration must fail closed");
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("malformed"));
        }

        #[test]
        fn seqpacket_client_rejects_ancillary_file_descriptors() {
            let (client, server) = socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC,
            )
            .expect("create seqpacket pair");
            let file = std::fs::File::open("/dev/null").expect("open descriptor fixture");
            let rights = [file.as_fd()];
            let mut control_bytes = [0_u8; rustix::cmsg_space!(ScmRights(1))];
            let mut control = SendAncillaryBuffer::new(&mut control_bytes);
            assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
            let frame = 0_u32.to_le_bytes();
            let iov = [IoSlice::new(&frame)];
            sendmsg(&server, &iov, &mut control, SendFlags::empty()).expect("send ancillary frame");
            let mut socket = SeqpacketUnixSocket { fd: client };
            let error = socket
                .recv_frame()
                .expect_err("ancillary data must fail closed");
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("ancillary"));
        }
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
    fn wire_invalid_frame_preserves_the_validation_exit_surface() {
        let failure = ZoneContext::local_only().client_failure(
            ClientError::Remote {
                kind: resource_error_kind("wire-invalid-frame"),
                retry: RetryClass::Never,
            },
            OutputMode::Json,
        );
        assert_eq!(failure.exit_code, 2);
        assert!(failure.message.starts_with("resource-schema-invalid:"));
        assert!(!failure.admission_recovery);
    }

    #[test]
    fn expedited_deadlines_use_the_ten_second_reconcile_bound() {
        assert_eq!(
            ZoneContext::expedited_deadline(Some("10s")).unwrap(),
            Some(10_000)
        );
        assert!(ZoneContext::expedited_deadline(Some("10.001s")).is_err());
        assert!(ZoneContext::expedited_deadline(Some("11s")).is_err());
    }

    #[test]
    fn bounded_messages_observe_a_utf8_byte_ceiling() {
        let message = "é".repeat(4096);
        assert!(bounded_message(&message).len() <= 4096);
        assert!(bounded_message(&message).is_char_boundary(4096));
    }

    #[test]
    fn resource_spec_files_are_bounded_before_json_parsing() {
        let path = std::env::temp_dir().join(format!(
            "d2b-resource-spec-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, vec![b'x'; MAX_SPEC_BYTES + 1]).expect("write oversized spec");
        let error = read_spec(Some(&path), false).expect_err("oversized file must fail");
        let _ = fs::remove_file(path);
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("64 KiB"));
    }

    #[test]
    fn injected_context_adds_frozen_envelope_fields() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"items":[]}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/public.sock", client.clone())
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
    fn explicit_zone_changes_the_request_target_but_keeps_the_root_listener() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"items":[]}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("child", "/run/d2b/public.sock", client.clone()).unwrap();
        context
            .invoke(
                "List",
                json!({"resourceType":"Guest"}),
                ZoneContext::deadline(None).unwrap(),
                OutputMode::Json,
            )
            .unwrap();
        assert_eq!(
            context.public_socket_path(),
            Path::new("/run/d2b/public.sock")
        );
        let requests = client.requests.lock().unwrap();
        let request: Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(request["zoneRef"], "Zone/child");
    }

    #[test]
    fn injected_process_attach_uses_the_typed_zone_attach_operation() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"ok":true}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/public.sock", client.clone())
                .unwrap();
        let response = context
            .attach_process(
                ResourceRef::parse("EphemeralProcess/command").unwrap(),
                false,
                false,
                ZoneContext::deadline(Some("30s")).unwrap(),
                OutputMode::Json,
            )
            .unwrap();
        assert_eq!(response["attached"], true);
        let requests = client.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request: Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(request["method"], "Attach");
        assert_eq!(request["service"], "d2b.zone.v3");
        assert_eq!(request["sessionVerb"], "attach");
        assert_eq!(request["resourceRef"], "EphemeralProcess/command");
        assert!(!request.to_string().contains("OpenTerminal"));
        assert!(!request.to_string().contains("subject"));
        assert!(!request.to_string().contains("user"));
    }

    #[test]
    fn cli_attach_stream_closes_idempotently_and_refuses_unowned_bytes() {
        let stream = CliAttachStream::new(None);
        assert_eq!(
            block_on(stream.send(vec![1])).unwrap_err(),
            ClientError::ContractViolation
        );
        assert_eq!(
            block_on(stream.receive()).unwrap_err(),
            ClientError::ContractViolation
        );
        block_on(stream.close()).unwrap();
        block_on(stream.close()).unwrap();
        assert!(stream.closed.load(Ordering::Acquire));
    }

    #[test]
    fn cli_attach_stream_drop_sends_a_typed_cancel_frame() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let mut server = SeqpacketUnixSocket::from_owned_fd(server);
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&server.recv_frame().unwrap()).unwrap();
            assert_eq!(request.request_id, 1);
            assert!(matches!(request.request, NamedProcessStreamRequest::Cancel));
        });
        let stream = CliAttachStream::new(Some(Arc::new(Mutex::new(
            SeqpacketUnixSocket::from_owned_fd(client),
        ))));
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    fn cli_attach_stream_retries_partial_stdin_writes() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let mut server = SeqpacketUnixSocket::from_owned_fd(server);
            let first: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&server.recv_frame().unwrap()).unwrap();
            assert_eq!(first.request_id, 1);
            let NamedProcessStreamRequest::Stdin {
                offset,
                chunk_base64,
                eof,
            } = first.request
            else {
                panic!("expected stdin frame");
            };
            assert_eq!(offset, 0);
            assert!(!eof);
            assert_eq!(
                d2b_core::base64_codec::decode(&chunk_base64).unwrap(),
                b"abc"
            );
            server
                .send_frame(
                    &serde_json::to_vec(&NamedProcessStreamResponseFrame::new(
                        1,
                        NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                            accepted_len: 1,
                            next_offset: 1,
                            backpressured: true,
                            stdin_closed: false,
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap();
            let second: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&server.recv_frame().unwrap()).unwrap();
            assert_eq!(second.request_id, 2);
            let NamedProcessStreamRequest::Stdin {
                offset,
                chunk_base64,
                eof,
            } = second.request
            else {
                panic!("expected stdin frame");
            };
            assert_eq!(offset, 1);
            assert!(!eof);
            assert_eq!(
                d2b_core::base64_codec::decode(&chunk_base64).unwrap(),
                b"bc"
            );
            server
                .send_frame(
                    &serde_json::to_vec(&NamedProcessStreamResponseFrame::new(
                        2,
                        NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                            accepted_len: 2,
                            next_offset: 3,
                            backpressured: false,
                            stdin_closed: false,
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap();
        });
        let stream = CliAttachStream::new(Some(Arc::new(Mutex::new(
            SeqpacketUnixSocket::from_owned_fd(client),
        ))));
        block_on(stream.send(b"abc".to_vec())).unwrap();
        assert_eq!(*stream.stdin_offset.lock().unwrap(), 3);
        server.join().unwrap();
    }

    #[test]
    fn cli_attach_stream_delivers_final_bytes_then_reports_eof() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let mut server = SeqpacketUnixSocket::from_owned_fd(server);
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&server.recv_frame().unwrap()).unwrap();
            assert_eq!(request.request_id, 1);
            assert!(matches!(
                request.request,
                NamedProcessStreamRequest::Read {
                    stream: ExecStream::Stdout,
                    ..
                }
            ));
            server
                .send_frame(
                    &serde_json::to_vec(&NamedProcessStreamResponseFrame::new(
                        1,
                        NamedProcessStreamResponse::Output(ExecReadOutputResult {
                            data_base64: d2b_core::base64_codec::encode(b"done"),
                            next_offset: 4,
                            eof: true,
                            dropped_bytes: 0,
                            truncated: false,
                            timed_out: false,
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap();
        });
        let stream = CliAttachStream::new(Some(Arc::new(Mutex::new(
            SeqpacketUnixSocket::from_owned_fd(client),
        ))));
        assert_eq!(block_on(stream.receive()).unwrap(), b"done");
        assert_eq!(
            block_on(stream.receive()).unwrap_err(),
            ClientError::Cancelled
        );
        server.join().unwrap();
    }

    #[test]
    fn cli_attach_stream_never_interprets_stdin_as_resize_control() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let stdin = br#"{"type":"namedStreamResize","rows":1,"cols":1}"#.to_vec();
        let expected = stdin.clone();
        let server = std::thread::spawn(move || {
            let mut server = SeqpacketUnixSocket::from_owned_fd(server);
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&server.recv_frame().unwrap()).unwrap();
            let NamedProcessStreamRequest::Stdin {
                offset,
                chunk_base64,
                eof,
            } = request.request
            else {
                panic!("expected stdin frame");
            };
            assert_eq!(request.request_id, 1);
            assert_eq!(offset, 0);
            assert!(!eof);
            let data = d2b_core::base64_codec::decode(&chunk_base64).unwrap();
            assert_eq!(data, expected);
            server
                .send_frame(
                    &serde_json::to_vec(&NamedProcessStreamResponseFrame::new(
                        1,
                        NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                            accepted_len: data.len() as u64,
                            next_offset: data.len() as u64,
                            backpressured: false,
                            stdin_closed: false,
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap();
        });
        let stream = CliAttachStream::new(Some(Arc::new(Mutex::new(
            SeqpacketUnixSocket::from_owned_fd(client),
        ))));
        block_on(stream.send(stdin)).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn injected_process_attach_redacts_remote_reason() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response:
                br#"{"ok":false,"errorClass":"authorization-denied","message":"secret-subject"}"#
                    .to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/public.sock", client).unwrap();
        let error = context
            .attach_process(
                ResourceRef::parse("EphemeralProcess/command").unwrap(),
                false,
                false,
                ZoneContext::deadline(Some("30s")).unwrap(),
                OutputMode::Json,
            )
            .unwrap_err();
        assert_eq!(error.exit_code, 1);
        assert!(!error.message.contains("secret-subject"));
        assert!(
            !error
                .rendered_stderr
                .unwrap_or_default()
                .contains("secret-subject")
        );
    }

    #[test]
    fn canonical_call_policy_binds_zone_service_and_mutation_idempotency() {
        assert_eq!(operation_service("ZoneGet"), ZoneServiceKind::Zone);
        assert_eq!(
            operation_service("ResolveEndpoint"),
            ZoneServiceKind::Resource
        );
        assert!(matches!(
            owner_for_zone(&zone_path("local-root").unwrap()),
            ServiceOwner::ZoneLocal(_)
        ));
        assert!(matches!(
            owner_for_zone(&zone_path("work").unwrap()),
            ServiceOwner::Zone(_)
        ));

        let deadline = ZoneContext::deadline(Some("30s")).unwrap();
        let read = call_options(deadline, ResourceVerb::Get).unwrap();
        assert!(!read.metadata.has_idempotency_key());

        let write = call_options(deadline, ResourceVerb::UpdateSpec).unwrap();
        assert!(write.metadata.has_idempotency_key());
        assert_eq!(write.retry.max_attempts(), 1);
    }

    #[test]
    fn root_listener_selection_does_not_infer_a_zone_from_socket_paths() {
        let context = ZoneContext::local_only();
        assert_eq!(
            context.public_socket_path(),
            Path::new("/run/d2b/public.sock")
        );
        assert_eq!(context.zone_ref(), "Zone/local-root");
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
