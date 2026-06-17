//! Transport-neutral CLI-to-`nixlingd` daemon access (ADR 0032).
//!
//! The local binding intentionally speaks the existing public daemon wire:
//! AF_UNIX `SOCK_SEQPACKET`, one 4-byte little-endian length-prefixed JSON
//! body per packet, `hello` negotiation, then the current type-tagged `list`
//! request. The historical wire has no node id and no exact v2 state for
//! `pending-restart` or `unknown`; list entries are mapped to
//! [`WorkloadSummary`] with node id `local`, VM name as workload id, declared
//! graphics/USB/common VM capabilities, and a fail-closed `Unknown` → `Failed`
//! state conversion.

pub mod direct_tls;
pub mod relay;

use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream as StdUnixStream},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use nix::sys::socket::{connect, socket, AddressFamily, SockFlag, SockType, UnixAddr};
use nixling_constellation_core::{
    Capability, CapabilitySet, ErrorKind, NodeId, ProviderId, WorkloadId, WorkloadState,
    WorkloadSummary,
};
use nixling_constellation_provider::{
    error::{ProviderError, ProviderResult},
    provider::{DaemonAccessApi, DaemonAccessTransport},
    types::{DaemonAccessMode, SafeLabel, TransportSession, TransportTarget},
};
use nixling_ipc::{
    public_wire::{ListEntry, ListRequest, ListResponse, VmLifecycleState},
    FeatureFlag, Hello, HelloOk, HelloRejected, KnownFeatureFlag, SemverRange, MAX_FRAME_SIZE,
    PUBLIC_SOCKET_PATH,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use direct_tls::DirectTlsDaemonAccess;
pub use relay::RelayDaemonAccess;

/// Default daemon public socket used by the current CLI and `nixlingd`.
pub const DEFAULT_PUBLIC_SOCKET_PATH: &str = PUBLIC_SOCKET_PATH;
const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
const LOCAL_NODE_ID: &str = "local";

/// Local public-socket daemon access.
#[derive(Debug, Clone)]
pub struct LocalUnixDaemonAccess {
    socket_path: PathBuf,
    transport_id: ProviderId,
    node_id: NodeId,
}

impl LocalUnixDaemonAccess {
    /// Construct access using the framework default public socket.
    pub fn new() -> Self {
        Self::with_socket_path(DEFAULT_PUBLIC_SOCKET_PATH)
    }

    /// Construct access using an explicit public-socket path.
    pub fn with_socket_path(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
            transport_id: ProviderId::parse("local-unix-daemon-access")
                .expect("static provider id is valid"),
            node_id: NodeId::parse(LOCAL_NODE_ID).expect("static node id is valid"),
        }
    }

    /// The configured public-socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    async fn request(&self, request_type: &'static str, payload: &[u8]) -> ProviderResult<Vec<u8>> {
        let mut session = self
            .connect(TransportTarget {
                endpoint: LOCAL_NODE_ID.to_owned(),
            })
            .await?;
        let stream = session.stream_mut();

        let hello = encode_type_tagged_message(
            "hello",
            &Hello {
                client_version: SemverRange::new(DEFAULT_CLIENT_VERSION_RANGE).map_err(|err| {
                    ProviderError::new(
                        ErrorKind::VersionSkew,
                        format!("invalid daemon client version range: {err}"),
                    )
                })?,
                supported_features: daemon_supported_features(),
            },
        )?;
        send_frame(stream, &hello).await?;
        let hello_response = recv_frame(stream).await?;
        parse_hello_reply(&hello_response)?;

        send_frame(stream, payload).await?;
        let response = recv_frame(stream).await?;
        reject_error_frame(request_type, &response)?;
        Ok(response)
    }
}

impl Default for LocalUnixDaemonAccess {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DaemonAccessTransport for LocalUnixDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        self.transport_id.clone()
    }

    fn mode(&self) -> DaemonAccessMode {
        DaemonAccessMode::LocalUnix
    }

    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        let path = self.socket_path.clone();
        let stream = tokio::task::spawn_blocking(move || connect_seqpacket(&path))
            .await
            .map_err(|err| {
                ProviderError::new(
                    ErrorKind::GatewayUnavailable,
                    format!("local daemon socket connect task failed: {err}"),
                )
            })?
            .map_err(|err| {
                ProviderError::new(
                    ErrorKind::GatewayUnavailable,
                    format!("local daemon public socket unavailable: {}", err.kind()),
                )
            })?;
        let stream = tokio::net::UnixStream::from_std(stream).map_err(|err| {
            ProviderError::new(
                ErrorKind::GatewayUnavailable,
                format!("local daemon socket registration failed: {}", err.kind()),
            )
        })?;
        Ok(TransportSession::new(
            SafeLabel::new("local-unix-public-sock"),
            Box::new(stream),
        ))
    }
}

#[async_trait]
impl DaemonAccessApi for LocalUnixDaemonAccess {
    async fn vm_list(&self) -> ProviderResult<Vec<WorkloadSummary>> {
        let request = encode_type_tagged_message(
            "list",
            &ListRequest {
                env: None,
                vm: None,
            },
        )?;
        let response = self.request("list", &request).await?;
        let list = parse_list_response(&response)?;
        workload_summaries_from_list_response(list, &self.node_id)
    }
}

/// Map the current daemon list response into v2 workload summaries.
pub fn workload_summaries_from_list_response(
    response: ListResponse,
    node_id: &NodeId,
) -> ProviderResult<Vec<WorkloadSummary>> {
    response
        .vms
        .into_iter()
        .map(|entry| workload_summary_from_list_entry(entry, node_id))
        .collect()
}

/// Map one current daemon list entry into a v2 workload summary.
pub fn workload_summary_from_list_entry(
    entry: ListEntry,
    node_id: &NodeId,
) -> ProviderResult<WorkloadSummary> {
    let id = WorkloadId::parse(entry.vm.clone()).map_err(|err| {
        ProviderError::new(
            ErrorKind::InvalidTarget,
            format!("daemon list entry carried invalid VM id: {err}"),
        )
    })?;
    Ok(WorkloadSummary {
        id,
        node: node_id.clone(),
        state: workload_state_from_lifecycle(entry.lifecycle.state),
        capabilities: capabilities_from_list_entry(&entry),
    })
}

fn capabilities_from_list_entry(entry: &ListEntry) -> CapabilitySet {
    let mut capabilities = CapabilitySet::empty()
        .with(Capability::Lifecycle)
        .with(Capability::Virtiofs)
        .with(Capability::Vsock);
    if entry.graphics {
        capabilities = capabilities
            .with(Capability::WindowForwarding)
            .with(Capability::GpuAccel);
    }
    if entry.usbip {
        capabilities = capabilities.with(Capability::Usb);
    }
    capabilities
}

fn workload_state_from_lifecycle(state: VmLifecycleState) -> WorkloadState {
    match state {
        VmLifecycleState::Stopped => WorkloadState::Stopped,
        VmLifecycleState::Starting => WorkloadState::Starting,
        VmLifecycleState::Booted | VmLifecycleState::Running => WorkloadState::Running,
        VmLifecycleState::Stopping => WorkloadState::Stopping,
        VmLifecycleState::Restarting => WorkloadState::Starting,
        VmLifecycleState::Failed | VmLifecycleState::Unknown => WorkloadState::Failed,
    }
}

fn connect_seqpacket(path: &Path) -> io::Result<StdUnixStream> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(nix_err_to_io)?;
    let addr = UnixAddr::new(path).map_err(nix_err_to_io)?;
    connect(fd.as_raw_fd(), &addr).map_err(nix_err_to_io)?;
    let stream = StdUnixStream::from(fd);
    stream.set_nonblocking(true)?;
    Ok(stream)
}

async fn send_frame(
    stream: &mut dyn nixling_constellation_provider::types::ByteStream,
    payload: &[u8],
) -> ProviderResult<()> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "daemon request frame exceeds public socket limit",
        ));
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    let written = stream.write(&frame).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("daemon socket write failed: {}", err.kind()),
        )
    })?;
    if written != frame.len() {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon socket accepted a short seqpacket write",
        ));
    }
    Ok(())
}

async fn recv_frame(
    stream: &mut dyn nixling_constellation_provider::types::ByteStream,
) -> ProviderResult<Vec<u8>> {
    let mut buffer = vec![0_u8; MAX_FRAME_SIZE + 4];
    let received = stream.read(&mut buffer).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("daemon socket read failed: {}", err.kind()),
        )
    })?;
    if received < 4 {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned a short public socket frame",
        ));
    }
    let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
    if expected > MAX_FRAME_SIZE || expected + 4 > received {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned a malformed public socket frame",
        ));
    }
    Ok(buffer[4..4 + expected].to_vec())
}

fn encode_type_tagged_message<T>(type_name: &str, message: &T) -> ProviderResult<Vec<u8>>
where
    T: Serialize,
{
    let mut value = serde_json::to_value(message).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to encode daemon request: {err}"),
        )
    })?;
    value
        .as_object_mut()
        .ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "daemon request payload must encode as a JSON object",
            )
        })?
        .insert("type".to_owned(), Value::String(type_name.to_owned()));
    serde_json::to_vec(&value).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to serialize daemon request: {err}"),
        )
    })
}

fn daemon_supported_features() -> Vec<FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
    ]
}

fn parse_hello_reply(response: &[u8]) -> ProviderResult<HelloOk> {
    let value = parse_json(response, "hello reply")?;
    match frame_type(&value)? {
        "helloOk" => decode_value::<HelloOkFrame>(value).map(|frame| frame.payload),
        "helloRejected" => decode_value::<HelloRejectedFrame>(value)
            .and_then(|frame| Err(provider_error_from_daemon_error(frame.error))),
        "error" => decode_value::<ErrorFrame>(value)
            .and_then(|frame| Err(provider_error_from_daemon_error(frame.error))),
        _ => Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned an unexpected hello reply",
        )),
    }
}

fn parse_list_response(response: &[u8]) -> ProviderResult<ListResponse> {
    let value = parse_json(response, "list response")?;
    if frame_type(&value)? != "listResponse" {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned an unexpected list reply",
        ));
    }
    decode_value::<ListResponseFrame>(value).map(|frame| ListResponse { vms: frame.vms })
}

fn reject_error_frame(request_type: &'static str, response: &[u8]) -> ProviderResult<()> {
    let value = parse_json(response, request_type)?;
    if frame_type(&value)? == "error" {
        let frame = decode_value::<ErrorFrame>(value)?;
        return Err(provider_error_from_daemon_error(frame.error));
    }
    Ok(())
}

fn parse_json(bytes: &[u8], context: &'static str) -> ProviderResult<Value> {
    serde_json::from_slice(bytes).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to parse daemon {context}: {err}"),
        )
    })
}

fn frame_type(value: &Value) -> ProviderResult<&str> {
    value.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon frame missing type discriminator",
        )
    })
}

fn decode_value<T>(value: Value) -> ProviderResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to decode daemon frame: {err}"),
        )
    })
}

fn provider_error_from_daemon_error(error: DaemonErrorEnvelope) -> ProviderError {
    let kind = match error.kind.as_str() {
        "authz-not-a-launcher" | "authz-audit-requires-admin" => ErrorKind::Unauthorized,
        "wire-version-mismatch" => ErrorKind::VersionSkew,
        "wire-frame-too-large" => ErrorKind::FrameTooLarge,
        "wire-unknown-field" | "wire-ifname-invalid" | "wire-malformed-json" => {
            ErrorKind::MalformedFrame
        }
        "broker-unimplemented" => ErrorKind::UnsupportedFeature,
        "broker-validation-failed" => ErrorKind::ProviderAllocationFailed,
        "manifest-parse-error" | "manifest-version-mismatch" => ErrorKind::MalformedFrame,
        "internal-io" | "bundle-tampered" => ErrorKind::GatewayUnavailable,
        _ => ErrorKind::MalformedFrame,
    };
    ProviderError::new(kind, error.message)
}

fn nix_err_to_io(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: HelloOk,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: HelloRejected,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorFrame {
    #[serde(rename = "type")]
    _type_name: String,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonErrorEnvelope {
    kind: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    vms: Vec<ListEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::{
        sys::socket::{accept4, bind, listen, send, Backlog, MsgFlags},
        unistd::close,
    };
    use nixling_ipc::{
        public_wire::{PublicVmServices, RuntimeSummary, VmLifecycle},
        Version,
    };
    use std::{
        fs,
        os::fd::RawFd,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    static TEST_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn local_mode_is_implemented() {
        let access = LocalUnixDaemonAccess::new();
        assert_eq!(access.mode(), DaemonAccessMode::LocalUnix);
        assert!(access.mode().is_implemented());
        assert_eq!(access.socket_path(), Path::new(DEFAULT_PUBLIC_SOCKET_PATH));
    }

    #[tokio::test]
    async fn relay_and_direct_tls_fail_closed() {
        let target = TransportTarget {
            endpoint: "example".to_owned(),
        };

        let relay = RelayDaemonAccess::new();
        assert_eq!(relay.mode(), DaemonAccessMode::Relay);
        assert!(!relay.mode().is_implemented());
        let relay_error = relay
            .connect(target.clone())
            .await
            .expect_err("relay is not implemented");
        assert_eq!(relay_error.kind(), ErrorKind::UnsupportedFeature);

        let direct_tls = DirectTlsDaemonAccess::new();
        assert_eq!(direct_tls.mode(), DaemonAccessMode::DirectTls);
        assert!(!direct_tls.mode().is_implemented());
        let direct_tls_error = direct_tls
            .connect(target)
            .await
            .expect_err("direct-tls is not implemented");
        assert_eq!(direct_tls_error.kind(), ErrorKind::UnsupportedFeature);
    }

    #[test]
    fn mapping_preserves_current_list_semantics_that_fit_workload_summary() {
        let entry = list_entry("work", VmLifecycleState::Running, true, true);
        let node = NodeId::parse("local").expect("node id");
        let summary = workload_summary_from_list_entry(entry, &node).expect("summary");

        assert_eq!(summary.id.as_str(), "work");
        assert_eq!(summary.node.as_str(), "local");
        assert_eq!(summary.state, WorkloadState::Running);
        assert!(summary.capabilities.has(Capability::Lifecycle));
        assert!(summary.capabilities.has(Capability::Virtiofs));
        assert!(summary.capabilities.has(Capability::Vsock));
        assert!(summary.capabilities.has(Capability::WindowForwarding));
        assert!(summary.capabilities.has(Capability::GpuAccel));
        assert!(summary.capabilities.has(Capability::Usb));
    }

    #[tokio::test]
    async fn local_unix_vm_list_round_trips_over_seqpacket_public_wire() {
        let socket_path = test_socket_path("vmlist");
        let listener = bind_seqpacket_listener(&socket_path);
        let entry = list_entry("work", VmLifecycleState::Running, true, true);
        let server = thread::spawn({
            let response_entry = entry.clone();
            move || serve_one_list_round_trip(listener, response_entry)
        });

        let access = LocalUnixDaemonAccess::with_socket_path(&socket_path);
        let summaries = access.vm_list().await.expect("vm_list response");

        server
            .join()
            .expect("server thread")
            .expect("server exchange");
        let _ = fs::remove_file(&socket_path);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id.as_str(), "work");
        assert_eq!(summaries[0].node.as_str(), "local");
        assert_eq!(summaries[0].state, WorkloadState::Running);
        assert!(summaries[0].capabilities.has(Capability::Lifecycle));
        assert!(summaries[0].capabilities.has(Capability::WindowForwarding));
        assert!(summaries[0].capabilities.has(Capability::Usb));
    }

    fn list_entry(vm: &str, state: VmLifecycleState, graphics: bool, usbip: bool) -> ListEntry {
        ListEntry {
            env: Some("dev".to_owned()),
            graphics,
            is_net_vm: false,
            lifecycle: VmLifecycle {
                pending_restart: false,
                state,
            },
            name: vm.to_owned(),
            runtime: RuntimeSummary {
                detail: "running".to_owned(),
            },
            services: PublicVmServices {
                gpu: graphics.then(|| format!("nixling-{vm}-gpu.service")),
                microvm: format!("microvm@{vm}.service"),
                nixling: format!("nixling@{vm}.service"),
                snd: None,
                swtpm: None,
                video: None,
                virtiofsd: format!("virtiofsd-{vm}.service"),
            },
            ssh_user: Some("alice".to_owned()),
            static_ip: Some("10.20.0.10".to_owned()),
            tpm: false,
            usbip,
            vm: vm.to_owned(),
        }
    }

    fn bind_seqpacket_listener(path: &Path) -> std::os::fd::OwnedFd {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create socket parent");
        }
        let _ = fs::remove_file(path);
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("listener socket");
        let addr = UnixAddr::new(path).expect("unix addr");
        bind(listener.as_raw_fd(), &addr).expect("bind listener");
        listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");
        listener
    }

    fn serve_one_list_round_trip(
        listener: std::os::fd::OwnedFd,
        response_entry: ListEntry,
    ) -> io::Result<()> {
        let accepted =
            accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).map_err(nix_err_to_io)?;
        let result = (|| -> io::Result<()> {
            let hello = recv_test_frame(accepted)?;
            let hello: Value = serde_json::from_slice(&hello)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));

            let hello_reply = encode_type_tagged_message(
                "helloOk",
                &HelloOk {
                    server_version: Version::new("0.4.0").expect("server version"),
                    selected_version: Version::new("0.4.0").expect("selected version"),
                    capabilities: daemon_supported_features(),
                },
            )
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            send_test_frame(accepted, &hello_reply)?;

            let request = recv_test_frame(accepted)?;
            let request: Value = serde_json::from_slice(&request)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(request.get("type").and_then(Value::as_str), Some("list"));
            assert_eq!(request.get("env"), Some(&Value::Null));
            assert_eq!(request.get("vm"), Some(&Value::Null));

            let mut response = serde_json::json!({ "type": "listResponse" });
            response["vms"] = serde_json::to_value(vec![response_entry])
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let response = serde_json::to_vec(&response)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            send_test_frame(accepted, &response)
        })();
        close(accepted).map_err(nix_err_to_io)?;
        result
    }

    fn test_socket_path(prefix: &str) -> PathBuf {
        let counter = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".t")
            .join(format!("{prefix}-{}-{counter}.s", std::process::id()))
    }

    fn recv_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_SIZE + 4];
        let received =
            nix::sys::socket::recv(fd, &mut buffer, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if received < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_SIZE || expected + 4 > received {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }

    fn send_test_frame(fd: RawFd, payload: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let sent = send(fd, &frame, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }
}
