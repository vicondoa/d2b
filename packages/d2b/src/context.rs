//! Zone selection, request bounds, and the small transport facade used by the
//! native CLI.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    future::Future,
    io::{self, Read as _},
    os::fd::{AsRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts::{
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected, KnownFeatureFlag,
    SemverRange,
};
use d2b_contracts_control::public_wire::{
    ExecReadOutputResult, ExecStream, ExecWriteStdinResult, NamedProcessStreamErrorKind,
    NamedProcessStreamRequest, NamedProcessStreamRequestFrame, NamedProcessStreamResponse,
    NamedProcessStreamResponseFrame,
};
use d2b_contracts_resource::v3::identity::{
    STANDARD_RESOURCE_TYPES, V3_CONVERTED_RESOURCE_TYPES,
};
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
use nix::errno::Errno;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::unix::AsyncFd;
use tokio::sync::Mutex as AsyncMutex;

use crate::runtime::{block_on, inside_runtime};
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
/// The deadline for one interactive named-stream round trip.
///
/// The terminal loop reports a named transport failure when a peer does not
/// answer within this bound, instead of blocking the operator's terminal.
pub(crate) const SHELL_STREAM_IO_DEADLINE_MS: u64 = 5_000;
/// The framed-envelope width: a 4-byte little-endian length prefix.
const FRAME_PREFIX_BYTES: usize = 4;

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

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn read_json_file<T>(path: &Path) -> Result<T, io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    let data = fs::read(path)?;
    serde_json::from_slice(&data).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
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
    block_on(probe_socket_within(
        path,
        Duration::from_millis(LOCAL_HANDSHAKE_DEADLINE_MS),
    ))
}

async fn probe_socket_within(path: &Path, budget: Duration) -> Result<SocketProbe, CliFailure> {
    let socket = CliSocket::connect(path, budget).await.map_err(|error| {
        CliFailure::new(
            1,
            format!(
                "zone-unavailable: failed to connect to {}: {error}",
                path.display()
            ),
        )
    })?;
    let payload = daemon_hello_frame("hello")?;
    socket.send_frame(&payload, budget).await.map_err(|error| {
        CliFailure::new(
            1,
            format!("exec-transport-error: failed to send hello frame: {error}"),
        )
    })?;
    let response = socket.recv_frame(budget).await.map_err(|error| {
        CliFailure::new(
            1,
            format!("exec-transport-error: failed to receive hello reply: {error}"),
        )
    })?;
    let hello = parse_hello_reply(&response)?;
    socket.close();
    Ok(SocketProbe {
        reachable: true,
        version: Some(hello.selected_version.as_str().to_owned()),
    })
}

/// Connect once to `path` with the handshake deadline, for reachability
/// checks that do not need a session.
pub(crate) fn socket_connectable(path: &Path) -> io::Result<()> {
    let socket = block_on(CliSocket::connect(
        path,
        Duration::from_millis(LOCAL_HANDSHAKE_DEADLINE_MS),
    ))?;
    socket.close();
    Ok(())
}

/// One end of the CLI's framed JSON protocol over a non-blocking seqpacket
/// socket.
///
/// Readiness is tokio's [`AsyncFd`]: the descriptor is registered with the
/// process runtime's reactor and every syscall below is non-blocking, so no
/// CLI thread parks in the kernel on this path. The CLI envelope is unchanged
/// (one datagram per frame, a 4-byte little-endian length prefix and one JSON
/// body), and each operation carries an explicit deadline whose expiry
/// surfaces as [`io::ErrorKind::TimedOut`] for the caller to name.
pub(crate) struct CliSocket {
    fd: AsyncFd<OwnedFd>,
}

impl CliSocket {
    fn from_owned_fd(fd: OwnedFd) -> io::Result<Self> {
        Ok(Self {
            fd: AsyncFd::new(fd)?,
        })
    }

    /// Connect to `path`, bounded by `budget`.
    ///
    /// AF_UNIX `connect` completes synchronously except when the listener's
    /// queue is full, where a non-blocking connect refuses with `EAGAIN`; the
    /// retry loop stays inside the budget rather than parking the thread in
    /// the kernel on a wedged listener.
    #[allow(clippy::disallowed_methods, reason = "CLI-only path")]
    pub(crate) async fn connect(path: &Path, budget: Duration) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .map_err(nix_err_to_io)?;
        let addr = UnixAddr::new(path).map_err(nix_err_to_io)?;
        let deadline = Instant::now() + budget;
        let mut delay = Duration::from_millis(1);
        loop {
            match connect(fd.as_raw_fd(), &addr) {
                Ok(()) => break,
                // A full accept queue or an interrupted attempt: retry.
                Err(Errno::EAGAIN | Errno::EINTR | Errno::EINPROGRESS) => {
                    let now = Instant::now();
                    let Some(remaining) = deadline.checked_duration_since(now) else {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "connect to {} exceeded {}ms",
                                path.display(),
                                budget.as_millis()
                            ),
                        ));
                    };
                    tokio::time::sleep(delay.min(remaining)).await;
                    delay = (delay * 2).min(Duration::from_millis(25));
                }
                // A prior attempt that did complete: the socket is connected.
                Err(Errno::EISCONN) => break,
                Err(error) => return Err(nix_err_to_io(error)),
            }
        }
        Self::from_owned_fd(fd)
    }

    pub(crate) fn close(&self) {
        let _ = rustix::net::shutdown(self.fd.get_ref(), rustix::net::Shutdown::ReadWrite);
    }

    pub(crate) async fn send_frame(&self, payload: &[u8], budget: Duration) -> io::Result<()> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds 1 MiB limit",
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + FRAME_PREFIX_BYTES);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        match tokio::time::timeout(budget, self.write_frame(&frame)).await {
            Ok(result) => result,
            Err(_) => Err(deadline_error("send", budget)),
        }
    }

    pub(crate) async fn recv_frame(&self, budget: Duration) -> io::Result<Vec<u8>> {
        let frame = match tokio::time::timeout(budget, self.read_frame()).await {
            Ok(result) => result?,
            Err(_) => return Err(deadline_error("receive", budget)),
        };
        if frame.len() < FRAME_PREFIX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(frame[..FRAME_PREFIX_BYTES].try_into().expect("prefix"));
        if expected as usize > MAX_FRAME_BYTES
            || expected as usize + FRAME_PREFIX_BYTES != frame.len()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(frame[FRAME_PREFIX_BYTES..].to_vec())
    }

    /// Send one datagram: a seqpacket send is atomic, so a partial write is a
    /// protocol failure rather than a retry.
    async fn write_frame(&self, frame: &[u8]) -> io::Result<()> {
        loop {
            let mut ready = self.fd.writable().await?;
            match ready.try_io(|inner| {
                rustix::net::send(
                    inner.get_ref(),
                    frame,
                    rustix::net::SendFlags::DONTWAIT | rustix::net::SendFlags::NOSIGNAL,
                )
                .map_err(io::Error::from)
            }) {
                Ok(Ok(sent)) if sent == frame.len() => return Ok(()),
                Ok(Ok(sent)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        format!("short write on seqpacket socket: {sent} of {}", frame.len()),
                    ));
                }
                Ok(Err(error)) => return Err(error),
                // Spurious readiness: re-arm and wait again.
                Err(_) => continue,
            }
        }
    }

    /// Receive one datagram, refusing ancillary data and oversized frames.
    async fn read_frame(&self) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + FRAME_PREFIX_BYTES];
        loop {
            let mut ready = self.fd.readable().await?;
            match ready.try_io(|inner| {
                let mut iov = [rustix::io::IoSliceMut::new(&mut buffer)];
                let mut control_bytes = [0_u8; rustix::cmsg_space!(ScmRights(1))];
                let mut control = rustix::net::RecvAncillaryBuffer::new(&mut control_bytes);
                let received = loop {
                    match rustix::net::recvmsg(
                        inner.get_ref(),
                        &mut iov,
                        &mut control,
                        rustix::net::RecvFlags::DONTWAIT | rustix::net::RecvFlags::CMSG_CLOEXEC,
                    ) {
                        Err(rustix::io::Errno::INTR) => continue,
                        result => break result.map_err(io::Error::from),
                    }
                }?;
                if control.drain().next().is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ancillary data is not permitted on the CLI transport",
                    ));
                }
                if received.flags.contains(rustix::net::RecvFlags::TRUNC) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "oversized seqpacket frame",
                    ));
                }
                if received.bytes == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "peer closed the socket",
                    ));
                }
                buffer.truncate(received.bytes);
                Ok(std::mem::take(&mut buffer))
            }) {
                Ok(result) => return result,
                // Spurious readiness: re-arm and wait again.
                Err(_) => continue,
            }
        }
    }
}

fn deadline_error(operation: &str, budget: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("socket {operation} exceeded {}ms", budget.as_millis()),
    )
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
        block_on(run_shell_session(stream))
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
            ClientError::RetryBackoffUnavailable => (
                "zone-unavailable",
                "Zone request retry backoff was unavailable",
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

/// Terminal ownership for one interactive shell attachment.
///
/// This loop is the *only* reader of the terminal while it runs: the guard
/// holds raw mode, [`TerminalInput`] owns readiness on a duplicate of stdin,
/// and every branch below waits through the runtime rather than reading
/// synchronously. Adding a blocking read anywhere in this file would fight
/// this owner, which is why the loop is the single entry point.
///
/// crossterm's `EventStream` is the canonical async reader for a *line* UI
/// (and `reedline` for a REPL); this path is neither. It forwards raw bytes to
/// the guest's PTY, so re-encoding keystrokes as `Event`s would drop escape
/// sequences, IME composition, and paste fidelity. The established primitive
/// for raw terminal bytes is readiness on the tty descriptor, which is what
/// [`TerminalInput`] uses.
async fn run_shell_session(
    stream: d2b_resource_client::ProcessAttachStream<CliAttachStream>,
) -> Result<(), CliFailure> {
    let mut guard = crate::exec_client::FdStateGuard::enter(true, true)
        .map_err(|_| CliFailure::new(69, "shell terminal setup failed"))?;
    let mut host = crate::exec_client::RealHostIo;
    let mut signals = crate::exec_client::install_signals()
        .map_err(|_| CliFailure::new(69, "shell signal setup failed"))?;
    let terminal =
        TerminalInput::open().map_err(|_| CliFailure::new(69, "shell terminal setup failed"))?;
    let mut input = [0_u8; d2b_contracts_control::public_wire::EXEC_MAX_CHUNK_BYTES as usize];
    loop {
        tokio::select! {
            // Every branch below reuses this structure: the selected future
            // completes, the others are dropped, and the body runs to
            // completion - so a terminal read can never be cancelled between
            // "bytes were read" and "bytes were sent".
            _ = signals.waiter() => {
                for signal in crate::terminal_client::TerminalSignalSource::drain(&mut signals) {
                    match signal {
                        crate::exec_client::ExecSignal::Winch => {
                            if let Some(size) = shell_terminal_size(&host) {
                                let _ = stream.resize(size).await;
                            }
                        }
                        crate::exec_client::ExecSignal::Hangup
                        | crate::exec_client::ExecSignal::Terminate
                        | crate::exec_client::ExecSignal::Stop
                        | crate::exec_client::ExecSignal::Interrupt
                        | crate::exec_client::ExecSignal::Quit => {
                            let _ = stream.cancel().await;
                            guard.restore();
                            return Ok(());
                        }
                    }
                }
            }
            read = terminal.read(&mut input) => {
                match read {
                    Ok(0) => {
                        let _ = stream.close().await;
                        guard.restore();
                        return Ok(());
                    }
                    Ok(read) => {
                        stream
                            .send(&input[..read])
                            .await
                            .map_err(|error| shell_failure("shell input transport failed", error))?;
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => {
                        let _ = stream.cancel().await;
                        guard.restore();
                        return Err(CliFailure::new(69, "shell input failed"));
                    }
                }
            }
            output = stream.receive() => {
                match output {
                    Ok(output) => {
                        if !output.is_empty() {
                            host.write_stdout(&output)
                                .map_err(|_| CliFailure::new(69, "shell output failed"))?;
                        }
                    }
                    Err(ClientError::Cancelled) => {
                        guard.restore();
                        return Ok(());
                    }
                    Err(error) => {
                        guard.restore();
                        return Err(shell_failure("shell output transport failed", error));
                    }
                }
            }
        }
    }
}

/// Readiness-driven raw terminal input for the interactive shell.
///
/// The descriptor is a duplicate of stdin sharing its open file description,
/// so the guard's non-blocking flag applies to both while the original stays
/// owned by the process.
struct TerminalInput {
    fd: AsyncFd<OwnedFd>,
}

impl TerminalInput {
    fn open() -> io::Result<Self> {
        let fd = rustix::io::dup(rustix::stdio::stdin()).map_err(io::Error::from)?;
        Ok(Self {
            fd: AsyncFd::new(fd)?,
        })
    }

    async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ready = self.fd.readable().await?;
            match ready.try_io(|inner| {
                loop {
                    match rustix::io::read(inner.get_ref(), buf) {
                        Err(rustix::io::Errno::INTR) => continue,
                        result => return result.map_err(io::Error::from),
                    }
                }
            }) {
                Ok(result) => return result,
                // Spurious readiness (or a blocking read another handler
                // consumed): re-arm and wait again.
                Err(_) => continue,
            }
        }
    }
}

fn shell_terminal_size(host: &crate::exec_client::RealHostIo) -> Option<TerminalSize> {
    let (rows, cols) = host.window_size()?;
    u16::try_from(rows)
        .ok()
        .zip(u16::try_from(cols).ok())
        .and_then(|(rows, cols)| TerminalSize::new(rows, cols).ok())
}

/// Name one interactive shell transport failure.
///
/// A wedged peer must read as a bounded, named refusal (with its class) and
/// never as a hung terminal.
fn shell_failure(step: &str, error: ClientError) -> CliFailure {
    let class = match error {
        ClientError::DeadlineExpired => "deadline-exceeded",
        _ => "exec-transport-error",
    };
    CliFailure::new(69, format!("{class}: {step}"))
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

    async fn connect_now(
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
        let socket = CliSocket::connect(&self.socket_path, self.handshake_timeout)
            .await
            .map_err(classify_client_io_error)?;
        let hello = daemon_hello_frame("hello").map_err(|_| ClientError::ContractViolation)?;
        socket
            .send_frame(&hello, self.handshake_timeout)
            .await
            .map_err(classify_client_io_error)?;
        let hello_reply = socket
            .recv_frame(self.handshake_timeout)
            .await
            .map_err(classify_client_io_error)?;
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
                socket: Some(Arc::new(socket)),
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
        self.connect_now(target, service)
    }
}

#[derive(Clone)]
struct CliConnectedSession {
    zone_name: String,
    service: ZoneServiceKind,
    operation: String,
    session_verb: Option<String>,
    socket: Option<Arc<CliSocket>>,
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
/// close messages on the admitted named stream. Every method awaits one
/// bounded round trip on the pump: no terminal read and no kernel-blocking
/// call happens here, so a wedged peer surfaces as a named deadline instead
/// of a hung CLI.
struct CliAttachStream {
    closed: AtomicBool,
    teardown_sent: AtomicBool,
    eof: AtomicBool,
    socket: Option<Arc<CliSocket>>,
    /// Serializes whole request/response exchanges: the socket carries one
    /// frame stream, and the daemon answers requests in order.
    round_trip_guard: AsyncMutex<()>,
    next_request_id: AtomicU64,
    stdin_offset: AsyncMutex<u64>,
    stdout_offset: AtomicU64,
    control_sequence: AtomicU64,
}

impl CliAttachStream {
    fn new(socket: Option<Arc<CliSocket>>) -> Self {
        Self {
            closed: AtomicBool::new(false),
            teardown_sent: AtomicBool::new(false),
            eof: AtomicBool::new(false),
            socket,
            round_trip_guard: AsyncMutex::new(()),
            next_request_id: AtomicU64::new(1),
            stdin_offset: AsyncMutex::new(0),
            stdout_offset: AtomicU64::new(0),
            control_sequence: AtomicU64::new(1),
        }
    }

    fn io_budget(&self) -> Duration {
        Duration::from_millis(SHELL_STREAM_IO_DEADLINE_MS)
    }
}

impl NamedStreamTransport for CliAttachStream {
    async fn send(&self, bytes: Vec<u8>) -> Result<(), ClientError> {
        self.send_stdin(&bytes).await
    }

    async fn resize(&self, size: TerminalSize) -> Result<(), ClientError> {
        match self
            .round_trip(
                NamedProcessStreamRequest::Resize {
                    control_seq: self.control_sequence.fetch_add(1, Ordering::AcqRel),
                    rows: u32::from(size.rows()),
                    cols: u32::from(size.cols()),
                },
                self.io_budget(),
            )
            .await?
        {
            NamedProcessStreamResponse::Delivered(_) => Ok(()),
            _ => Err(ClientError::ContractViolation),
        }
    }

    async fn receive(&self) -> Result<Vec<u8>, ClientError> {
        if self.eof.load(Ordering::Acquire) {
            return Err(ClientError::Cancelled);
        }
        match self
            .round_trip(
                NamedProcessStreamRequest::Read {
                    stream: ExecStream::Stdout,
                    offset: self.stdout_offset.load(Ordering::Acquire),
                    max_len: d2b_contracts_control::public_wire::EXEC_MAX_CHUNK_BYTES,
                    wait: true,
                    timeout_ms: 50,
                },
                self.io_budget(),
            )
            .await?
        {
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
        }
    }

    async fn close(&self) -> Result<(), ClientError> {
        self.closed.store(true, Ordering::Release);
        let result = if self.socket.is_some() {
            match self
                .round_trip(NamedProcessStreamRequest::Close, self.io_budget())
                .await
            {
                Ok(NamedProcessStreamResponse::Closed(_)) => Ok(()),
                Ok(_) => Err(ClientError::ContractViolation),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.teardown_sent.store(true, Ordering::Release);
        }
        result
    }

    async fn cancel(&self) -> Result<(), ClientError> {
        self.closed.store(true, Ordering::Release);
        let result = if self.socket.is_some() {
            match self
                .round_trip(NamedProcessStreamRequest::Cancel, self.io_budget())
                .await
            {
                Ok(NamedProcessStreamResponse::Closed(_))
                | Ok(NamedProcessStreamResponse::Delivered(_)) => Ok(()),
                Ok(_) => Err(ClientError::ContractViolation),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.teardown_sent.store(true, Ordering::Release);
        }
        result
    }
}

impl Drop for CliAttachStream {
    fn drop(&mut self) {
        let Some(socket) = self.socket.take() else {
            return;
        };
        if self.teardown_sent.swap(true, Ordering::AcqRel) {
            return;
        }
        // Best-effort teardown. Drop cannot await, so one bounded cancel round
        // trip is driven here - unless a future the runtime is already driving
        // is what dropped the stream, where a nested `block_on` would panic;
        // there the socket close still ends the named stream.
        if inside_runtime() {
            return;
        }
        let _ = block_on(stream_round_trip(
            &socket,
            &self.next_request_id,
            NamedProcessStreamRequest::Cancel,
            Duration::from_millis(SHELL_STREAM_IO_DEADLINE_MS),
        ));
    }
}

impl CliAttachStream {
    async fn send_stdin(&self, bytes: &[u8]) -> Result<(), ClientError> {
        let mut offset = self.stdin_offset.lock().await;
        let deadline = Instant::now() + self.io_budget();
        let mut consumed = 0;
        while consumed < bytes.len() {
            let response = self
                .round_trip(
                    NamedProcessStreamRequest::Stdin {
                        offset: *offset,
                        chunk_base64: d2b_core::base64_codec::encode(&bytes[consumed..]),
                        eof: false,
                    },
                    self.io_budget(),
                )
                .await?;
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
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            }
            *offset = next_offset;
            consumed += accepted;
        }
        Ok(())
    }

    async fn round_trip(
        &self,
        request: NamedProcessStreamRequest,
        budget: Duration,
    ) -> Result<NamedProcessStreamResponse, ClientError> {
        let socket = self.socket.as_ref().ok_or(ClientError::ContractViolation)?;
        // One exchange at a time: a second request would otherwise interleave
        // its frame with the one in flight on this single socket.
        let _guard = self.round_trip_guard.lock().await;
        stream_round_trip(socket, &self.next_request_id, request, budget).await
    }
}

/// One bounded request/response exchange on an established named stream.
///
/// A response nobody is waiting for - a call the terminal loop stopped
/// waiting on when another `select!` branch won - is skipped instead of
/// mis-correlated with the next request. Stream reads are offset-addressed,
/// so dropping that response costs one round trip and never data. The budget
/// bounds the whole exchange, skips included, so a peer that answers only in
/// stale frames still ends as a named deadline.
async fn stream_round_trip(
    socket: &CliSocket,
    next_request_id: &AtomicU64,
    request: NamedProcessStreamRequest,
    budget: Duration,
) -> Result<NamedProcessStreamResponse, ClientError> {
    let request_id = next_request_id.fetch_add(1, Ordering::AcqRel);
    if request_id == 0 {
        return Err(ClientError::ContractViolation);
    }
    let deadline = Instant::now() + budget;
    let frame = NamedProcessStreamRequestFrame::new(request_id, request);
    let bytes = serde_json::to_vec(&frame).map_err(|_| ClientError::ContractViolation)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or(ClientError::DeadlineExpired)?;
    socket
        .send_frame(&bytes, remaining)
        .await
        .map_err(classify_client_io_error)?;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ClientError::DeadlineExpired)?;
        let response = socket
            .recv_frame(remaining)
            .await
            .map_err(classify_client_io_error)?;
        let frame: NamedProcessStreamResponseFrame =
            serde_json::from_slice(&response).map_err(|_| ClientError::ContractViolation)?;
        if frame.request_id != request_id {
            continue;
        }
        return match frame.response {
            NamedProcessStreamResponse::Error(error) => Err(named_stream_client_error(error.kind)),
            response => Ok(response),
        };
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
        self.invoke(target, payload, relative_timeout_nanos)
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
        std::future::ready(Err(ClientError::ContractViolation))
    }
}

impl ConnectedSession for CliConnectedSession {
    type Stream = CliAttachStream;

    fn open_named_stream(
        &self,
        request: ProcessAttachOpenRequest,
        relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<Self::Stream, ClientError>> + Send {
        self.open_process_attach(request, relative_timeout_nanos)
    }
}

impl CliConnectedSession {
    async fn open_process_attach(
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
        )
        .await?;
        let is_shell = matches!(request.target(), ProcessAttachTarget::ShellSession { .. });
        let socket = if is_shell { self.socket.clone() } else { None };
        Ok(CliAttachStream::new(socket))
    }

    async fn invoke(
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
        // The caller's declared request lifetime is the bound; the protocol
        // ceiling caps the sentinel value a deadline-less caller passes.
        let budget = Duration::from_nanos(relative_timeout_nanos.max(1)).min(
            Duration::from_millis(d2b_resource_client::MAX_REQUEST_LIFETIME_MS),
        );
        #[cfg(test)]
        if let Some(client) = &self.injected {
            let response = client
                .invoke(&request, RequestDeadline(budget))
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
        let socket = self.socket.as_ref().ok_or(ClientError::TransportFailed)?;
        socket
            .send_frame(&request, budget)
            .await
            .map_err(classify_client_io_error)?;
        let response = socket
            .recv_frame(budget)
            .await
            .map_err(classify_client_io_error)?;
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

pub(crate) fn standard_resource_types() -> &'static [&'static str; 23] {
    &STANDARD_RESOURCE_TYPES
}

/// The resource types the managed plane serves: what a zone-wide read must
/// cover, since a type this catalog names but the caller cannot read is a
/// degraded read rather than an absent one.
pub(crate) fn converted_resource_types() -> &'static [&'static str; 36] {
    &V3_CONVERTED_RESOURCE_TYPES
}

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
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

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
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
    block_on(socket_reachable_within(
        path,
        Duration::from_millis(LOCAL_HANDSHAKE_DEADLINE_MS),
    ))
}

async fn socket_reachable_within(path: &Path, budget: Duration) -> bool {
    let Ok(socket) = CliSocket::connect(path, budget).await else {
        return false;
    };
    let Ok(hello) = daemon_hello_frame("hello") else {
        return false;
    };
    if socket.send_frame(&hello, budget).await.is_err() {
        return false;
    }
    let Ok(reply) = socket.recv_frame(budget).await else {
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
        | "debug-read-refused"
        | "debug-read-exhausted"
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
    use std::os::fd::AsFd as _;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockClient {
        requests: Mutex<Vec<Vec<u8>>>,
        response: Vec<u8>,
    }

    /// A non-blocking client [`CliSocket`] plus a blocking mock peer for the
    /// same one-frame-per-datagram envelope.
    fn test_socket_pair() -> (CliSocket, OwnedFd) {
        let (client, server) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("create seqpacket pair");
        let flags =
            rustix::fs::fcntl_getfl(&server).expect("server flags") - rustix::fs::OFlags::NONBLOCK;
        rustix::fs::fcntl_setfl(&server, flags).expect("mock peer reads blocking");
        let client = block_on(async move { CliSocket::from_owned_fd(client) })
            .expect("client socket registers with the runtime reactor");
        (client, server)
    }

    /// Read one CLI frame from the blocking mock peer.
    ///
    /// The envelope is one datagram per frame, so the read is sized for a whole
    /// frame: a prefix-sized read truncates the datagram and the kernel
    /// discards the body. The declared length must account for every byte the
    /// datagram carried, which is what makes a split frame fail here rather
    /// than pass as a truncated request.
    fn mock_recv_frame(fd: std::os::fd::BorrowedFd<'_>) -> Vec<u8> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + FRAME_PREFIX_BYTES];
        let read = loop {
            match rustix::io::read(fd, &mut buffer) {
                Ok(0) => panic!("mock peer closed"),
                Ok(read) => break read,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => panic!("mock peer read: {error}"),
            }
        };
        assert!(
            read >= FRAME_PREFIX_BYTES,
            "frame from mock peer is shorter than its length prefix: {read} bytes"
        );
        let declared =
            u32::from_le_bytes(buffer[..FRAME_PREFIX_BYTES].try_into().expect("prefix")) as usize;
        assert_eq!(
            declared + FRAME_PREFIX_BYTES,
            read,
            "one datagram must carry exactly one declared frame"
        );
        buffer.truncate(read);
        buffer.split_off(FRAME_PREFIX_BYTES)
    }

    /// Write one CLI frame from the blocking mock peer.
    fn mock_send_frame(fd: std::os::fd::BorrowedFd<'_>, payload: &[u8]) {
        let mut frame = Vec::with_capacity(payload.len() + FRAME_PREFIX_BYTES);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let mut offset = 0;
        while offset < frame.len() {
            match rustix::io::write(fd, &frame[offset..]) {
                Ok(0) => panic!("mock peer closed"),
                Ok(written) => offset += written,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => panic!("mock peer write: {error}"),
            }
        }
    }

    /// One frame leaves the client as exactly one datagram, so a reader that
    /// sizes its buffer to the length prefix loses the body: the kernel
    /// truncates the datagram and discards the rest. This pins both halves -
    /// the sender writes one datagram per frame, and a prefix-sized read finds
    /// nothing behind it.
    #[test]
    fn cli_socket_writes_one_datagram_per_frame() {
        let (client, server) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("create seqpacket pair");
        let client = block_on(async move { CliSocket::from_owned_fd(client) }).unwrap();
        let payload = br#"{"type":"namedStreamCancel","requestId":1}"#;
        block_on(client.send_frame(payload, Duration::from_millis(500))).unwrap();
        let mut prefix = [0_u8; FRAME_PREFIX_BYTES];
        let first = rustix::io::read(server.as_fd(), &mut prefix).expect("read the prefix");
        assert_eq!(first, FRAME_PREFIX_BYTES);
        assert_eq!(
            u32::from_le_bytes(prefix) as usize,
            payload.len(),
            "the prefix declares the payload the same datagram carries"
        );
        let mut payload_buffer = vec![0_u8; payload.len()];
        assert!(
            matches!(
                rustix::io::read(server.as_fd(), &mut payload_buffer),
                Err(rustix::io::Errno::AGAIN)
            ),
            "a prefix-sized read must have consumed the whole datagram"
        );
    }

    #[cfg(test)]
    mod transport_contract_tests {
        use super::{MAX_FRAME_BYTES, test_socket_pair};
        use crate::runtime::block_on;
        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
        use std::{
            io::IoSlice,
            os::fd::AsFd as _,
            time::{Duration, Instant},
        };

        #[test]
        fn cli_socket_rejects_oversized_declared_packets() {
            let (socket, server) = test_socket_pair();
            block_on(async {
                let outbound = socket
                    .send_frame(&vec![0_u8; MAX_FRAME_BYTES + 1], Duration::from_millis(100))
                    .await
                    .expect_err("outbound oversized frame must fail closed");
                assert_eq!(outbound.kind(), std::io::ErrorKind::InvalidInput);
                // Declare more than the frame bound in the four-byte prefix.
                let declared = (MAX_FRAME_BYTES + 1) as u32;
                let prefix = declared.to_le_bytes();
                let written = rustix::io::write(server.as_fd(), &prefix).expect("send prefix");
                assert_eq!(written, prefix.len());
                let error = socket
                    .recv_frame(Duration::from_millis(500))
                    .await
                    .expect_err("oversized declaration must fail closed");
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert!(error.to_string().contains("malformed"));
            });
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn cli_socket_rejects_ancillary_file_descriptors() {
            let (socket, server) = test_socket_pair();
            let file = std::fs::File::open("/dev/null").expect("open descriptor fixture");
            let rights = [file.as_fd()];
            let mut control_bytes = [0_u8; rustix::cmsg_space!(ScmRights(1))];
            let mut control = SendAncillaryBuffer::new(&mut control_bytes);
            assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
            let frame = 0_u32.to_le_bytes();
            let iov = [IoSlice::new(&frame)];
            sendmsg(server.as_fd(), &iov, &mut control, SendFlags::empty())
                .expect("send ancillary frame");
            let error = block_on(async {
                socket
                    .recv_frame(Duration::from_millis(500))
                    .await
                    .expect_err("ancillary data must fail closed")
            });
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("ancillary"));
        }

        #[test]
        fn cli_socket_reports_a_stalled_peer_as_a_bounded_deadline() {
            let (socket, server) = test_socket_pair();
            let started = Instant::now();
            let error = block_on(async {
                socket
                    .recv_frame(Duration::from_millis(100))
                    .await
                    .expect_err("a silent peer must not park the CLI")
            });
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the deadline must bound the wait, took {:?}",
                started.elapsed()
            );
            drop(server);
        }

        #[test]
        fn cli_socket_reports_a_closed_peer_with_the_unreachable_errno() {
            let (socket, server) = test_socket_pair();
            drop(server);
            let error = block_on(async {
                socket
                    .recv_frame(Duration::from_millis(500))
                    .await
                    .expect_err("a closed peer is a session loss")
            });
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
        }
    }

    impl SessionClient for MockClient {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn injected_context_adds_frozen_envelope_fields() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"items":[]}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/public.sock", client.clone()).unwrap();
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn injected_process_attach_uses_the_typed_zone_attach_operation() {
        let client = Arc::new(MockClient {
            requests: Mutex::new(Vec::new()),
            response: br#"{"ok":true}"#.to_vec(),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/public.sock", client.clone()).unwrap();
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cli_attach_stream_drop_sends_a_typed_cancel_frame() {
        let (client, server) = test_socket_pair();
        let server = std::thread::spawn(move || {
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
            assert_eq!(request.request_id, 1);
            assert!(matches!(request.request, NamedProcessStreamRequest::Cancel));
        });
        let stream = CliAttachStream::new(Some(Arc::new(client)));
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cli_attach_stream_retries_partial_stdin_writes() {
        let (client, server) = test_socket_pair();
        let server = std::thread::spawn(move || {
            let first: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
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
            mock_send_frame(
                server.as_fd(),
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
            );
            let second: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
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
            mock_send_frame(
                server.as_fd(),
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
            );
        });
        let stream = CliAttachStream::new(Some(Arc::new(client)));
        block_on(async {
            stream.send(b"abc".to_vec()).await.unwrap();
            assert_eq!(*stream.stdin_offset.lock().await, 3);
        });
        server.join().unwrap();
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cli_attach_stream_delivers_final_bytes_then_reports_eof() {
        let (client, server) = test_socket_pair();
        let server = std::thread::spawn(move || {
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
            assert_eq!(request.request_id, 1);
            assert!(matches!(
                request.request,
                NamedProcessStreamRequest::Read {
                    stream: ExecStream::Stdout,
                    ..
                }
            ));
            mock_send_frame(
                server.as_fd(),
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
            );
        });
        let stream = CliAttachStream::new(Some(Arc::new(client)));
        block_on(async {
            assert_eq!(stream.receive().await.unwrap(), b"done");
            assert_eq!(stream.receive().await.unwrap_err(), ClientError::Cancelled);
        });
        server.join().unwrap();
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cli_attach_stream_never_interprets_stdin_as_resize_control() {
        let (client, server) = test_socket_pair();
        let stdin = br#"{"type":"namedStreamResize","rows":1,"cols":1}"#.to_vec();
        let expected = stdin.clone();
        let server = std::thread::spawn(move || {
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
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
            mock_send_frame(
                server.as_fd(),
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
            );
        });
        let stream = CliAttachStream::new(Some(Arc::new(client)));
        block_on(async {
            stream.send(stdin).await.unwrap();
        });
        server.join().unwrap();
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cli_attach_stream_names_a_stalled_round_trip_as_a_deadline() {
        // The interactive shell counts on this bound: a peer that stops
        // answering must end the round trip as a named deadline instead of
        // parking the operator's terminal in a read.
        let (client, server) = test_socket_pair();
        let server = std::thread::spawn(move || {
            // The read request is delivered, then deliberately unanswered.
            let request: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
            assert!(matches!(
                request.request,
                NamedProcessStreamRequest::Read { .. }
            ));
            // Answering teardown would hide the bound; the mock reads the
            // cancel and returns, closing the peer.
            let cancel: NamedProcessStreamRequestFrame =
                serde_json::from_slice(&mock_recv_frame(server.as_fd())).unwrap();
            assert!(matches!(cancel.request, NamedProcessStreamRequest::Cancel));
        });
let stream = CliAttachStream::new(Some(Arc::new(client)));
        let started = Instant::now();
        // The observable condition is the outcome itself:the round trip must
        // end as the named deadline (asserted below). The finite ceiling below
        // then guards the magnitude:the round trip is bounded by the advertised
        // 5s io budget, and a regression that inflates that budget (e.g., an
        // ms-misread-as-seconds change, or an order-of-magnitude inflation)
        // must fail on the wait itself, not merely return the right error kind
        // after taking far longer than the shell's bound should have allowed.



        // 60s is a 12x headroom over that budget: wide enough that scheduling
        // delay cannot trip it on any normally-loaded machine, finite enough
        // that the >12x inflation class fails on the measurement. (The server
        // thread above proves the teardown cancel was actually sent, and a
        // deadline that never fires would hang the test deterministically.)
        let error = block_on(stream.receive()).unwrap_err();
        let elapsed = started.elapsed();
        assert_eq!(error, ClientError::DeadlineExpired);
        assert!(
            elapsed < Duration::from_secs(60),
            "a silent peer must end the round trip within the 60s ceiling; took {elapsed:?}"
        );
        drop(stream);
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
        let context = ZoneContext::with_client("dev", "/run/d2b/public.sock", client).unwrap();
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
