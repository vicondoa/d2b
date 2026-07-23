//! Host-side authenticated guest-control Health probe.
//!
//! W11 stores authenticated health evidence only. It does not replace VM
//! lifecycle readiness and it does not expose exec.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use d2b_contracts::broker_wire::{
    GuestBootIdWire, GuestControlAuthPurpose, GuestControlDirection, GuestControlProofRole,
    GuestControlSignRequest, GuestControlSignResponse,
};
use d2b_contracts::guest_auth::{
    AUTH_NONCE_LEN, AUTH_TAG_LEN, AUTH_TRANSCRIPT_VERSION, GUEST_CONTROL_AUTH_PORT,
};
use d2b_contracts::guest_proto as pb;
use d2b_contracts::guest_wire::{GUEST_CONTROL_PROTOCOL_VERSION, READ_GUEST_FILE_MAX_BYTES};
use protobuf::{Message, MessageField};
use subtle::ConstantTimeEq;

use crate::guest_control_vsock::GuestControlConnectedStream;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestControlHealthError {
    TransportIo,
    Ttrpc,
    Signer,
    Protocol,
    AuthFailed,
    StaleSession,
    /// The absolute per-attempt deadline elapsed before (or during) an
    /// operation. Distinct from a generic transport/ttRPC failure so a
    /// genuine timeout surfaces as a timeout end-to-end (the
    /// `guest-control-timeout` config-sync error).
    Timeout,
}

/// An absolute-deadline budget for a single guest-control probe / config
/// read attempt.
///
/// Each sub-operation — connect, CONNECT-ACK, every ttRPC unary, and
/// every broker `sign` — draws `min(cap, deadline - now)` from the same
/// budget via [`AttemptBudget::next`], so the whole attempt is bounded
/// by its absolute deadline instead of running connect + N ttRPC + 2
/// signs each at the full per-attempt timeout (which let one attempt run
/// many multiples of the intended budget). [`AttemptBudget::next`]
/// returns `None` once the deadline is reached, which callers surface as
/// [`GuestControlHealthError::Timeout`].
#[derive(Clone, Copy, Debug)]
pub struct AttemptBudget {
    deadline: Instant,
    cap: Duration,
}

impl AttemptBudget {
    /// Budget with an explicit absolute deadline and a per-operation cap.
    pub fn new(deadline: Instant, cap: Duration) -> Self {
        Self { deadline, cap }
    }

    /// Budget whose deadline is `span` from now, with per-operation cap
    /// `cap`. A zero (or non-representable) span yields an immediately
    /// expired budget.
    pub fn from_now(span: Duration, cap: Duration) -> Self {
        let deadline = Instant::now()
            .checked_add(span)
            .unwrap_or_else(Instant::now);
        Self { deadline, cap }
    }

    /// The timeout to apply to the next sub-operation:
    /// `min(cap, deadline - now)` floored at 1ms, or `None` if the
    /// deadline has already passed.
    pub fn next(&self) -> Option<Duration> {
        let remaining = self.deadline.checked_duration_since(Instant::now())?;
        if remaining.is_zero() {
            return None;
        }
        Some(self.cap.min(remaining).max(Duration::from_millis(1)))
    }

    /// Whether the absolute deadline has elapsed.
    pub fn is_expired(&self) -> bool {
        self.next().is_none()
    }
}

#[derive(Debug, Clone)]
pub struct GuestControlHealthEvidence {
    pub vm_id: String,
    pub guest_boot_id: String,
    pub protocol_version: u32,
    pub capabilities_hash: String,
    pub health: pb::HealthResponse,
}

#[async_trait]
pub trait GuestControlRpc {
    async fn hello(
        &self,
        request: pb::HelloRequest,
    ) -> Result<pb::HelloResponse, GuestControlHealthError>;
    async fn authenticate(
        &self,
        request: pb::AuthenticateRequest,
    ) -> Result<pb::AuthenticateResponse, GuestControlHealthError>;
    async fn health(
        &self,
        request: pb::HealthRequest,
    ) -> Result<pb::HealthResponse, GuestControlHealthError>;
    async fn read_guest_file(
        &self,
        request: pb::ReadGuestFileRequest,
    ) -> Result<pb::ReadGuestFileResponse, GuestControlHealthError>;
    async fn usbip_import(
        &self,
        request: pb::UsbipImportRequest,
    ) -> Result<pb::UsbipImportResponse, GuestControlHealthError>;
    async fn usbip_status(
        &self,
        request: pb::UsbipStatusRequest,
    ) -> Result<pb::UsbipStatusResponse, GuestControlHealthError>;
    async fn activate_system_start(
        &self,
        request: pb::GuestActivationStartRequest,
    ) -> Result<pb::GuestActivationStartResponse, GuestControlHealthError> {
        let _ = request;
        Err(GuestControlHealthError::Protocol)
    }
    async fn activate_system_status(
        &self,
        request: pb::GuestActivationStatusRequest,
    ) -> Result<pb::GuestActivationStatusResponse, GuestControlHealthError> {
        let _ = request;
        Err(GuestControlHealthError::Protocol)
    }
    async fn audio_status(
        &self,
        request: pb::AudioStatusRequest,
    ) -> Result<pb::AudioStatusResponse, GuestControlHealthError> {
        let _ = request;
        Err(GuestControlHealthError::Protocol)
    }
    async fn audio_set(
        &self,
        request: pb::AudioSetRequest,
    ) -> Result<pb::AudioSetResponse, GuestControlHealthError> {
        let _ = request;
        Err(GuestControlHealthError::Protocol)
    }
}

pub trait GuestControlSigner {
    fn sign(
        &self,
        request: GuestControlSignRequest,
    ) -> Result<GuestControlSignResponse, GuestControlHealthError>;
}

pub fn connected_stream_to_ttrpc_socket(
    connected: GuestControlConnectedStream,
) -> Result<ttrpc::r#async::transport::Socket, GuestControlHealthError> {
    let socket = connected.into_socket();
    socket
        .set_read_timeout(None)
        .map_err(|_| GuestControlHealthError::TransportIo)?;
    socket
        .set_write_timeout(None)
        .map_err(|_| GuestControlHealthError::TransportIo)?;
    let fd: OwnedFd = socket.into();
    let stream = std::os::unix::net::UnixStream::from(fd);
    stream
        .set_nonblocking(true)
        .map_err(|_| GuestControlHealthError::TransportIo)?;
    let stream = tokio::net::UnixStream::from_std(stream)
        .map_err(|_| GuestControlHealthError::TransportIo)?;
    Ok(ttrpc::r#async::transport::Socket::new(stream))
}

pub struct TtrpcGuestControlClient {
    client: ttrpc::r#async::Client,
    budget: AttemptBudget,
}

impl TtrpcGuestControlClient {
    pub fn new(socket: ttrpc::r#async::transport::Socket, budget: AttemptBudget) -> Self {
        Self {
            client: ttrpc::r#async::Client::new(socket),
            budget,
        }
    }

    async fn unary<Req, Resp>(
        &self,
        method: &str,
        request: Req,
    ) -> Result<Resp, GuestControlHealthError>
    where
        Req: Message,
        Resp: Message + Default,
    {
        // Recompute the remaining attempt budget per call so connect +
        // every ttRPC share one absolute deadline. A passed deadline
        // surfaces as a timeout rather than blocking at the full timeout.
        let timeout = self.budget.next().ok_or(GuestControlHealthError::Timeout)?;
        self.unary_with_timeout(method, request, timeout).await
    }

    /// Issue a unary ttRPC with an EXPLICIT per-call timeout, independent of
    /// the handshake [`AttemptBudget`]. The exec session worker uses this for
    /// every proxied exec op so each op draws a FRESH absolute deadline rather
    /// than the one-shot establishment budget (which is exhausted by the time
    /// the first op runs).
    pub async fn unary_with_timeout<Req, Resp>(
        &self,
        method: &str,
        request: Req,
        timeout: Duration,
    ) -> Result<Resp, GuestControlHealthError>
    where
        Req: Message,
        Resp: Message + Default,
    {
        let timeout_nano = timeout.as_nanos().min(i64::MAX as u128) as i64;
        let mut payload = Vec::new();
        request
            .write_to_vec(&mut payload)
            .map_err(|_| GuestControlHealthError::Protocol)?;
        let response = self
            .client
            .request(ttrpc::Request {
                service: "d2b.guest.v1.GuestControl".to_owned(),
                method: method.to_owned(),
                timeout_nano,
                metadata: ttrpc::context::to_pb(HashMap::new()),
                payload,
                ..Default::default()
            })
            .await
            .map_err(map_ttrpc_request_error)?;
        Resp::parse_from_bytes(&response.payload).map_err(|_| GuestControlHealthError::Protocol)
    }
}

fn map_ttrpc_request_error(error: ttrpc::Error) -> GuestControlHealthError {
    match &error {
        ttrpc::Error::RpcStatus(status) if status.code() == ttrpc::Code::DEADLINE_EXCEEDED => {
            GuestControlHealthError::Timeout
        }
        ttrpc::Error::Others(message) if message.to_ascii_lowercase().contains("timeout") => {
            GuestControlHealthError::Timeout
        }
        _ => GuestControlHealthError::Ttrpc,
    }
}

#[async_trait]
impl GuestControlRpc for TtrpcGuestControlClient {
    async fn hello(
        &self,
        request: pb::HelloRequest,
    ) -> Result<pb::HelloResponse, GuestControlHealthError> {
        self.unary("Hello", request).await
    }

    async fn authenticate(
        &self,
        request: pb::AuthenticateRequest,
    ) -> Result<pb::AuthenticateResponse, GuestControlHealthError> {
        self.unary("Authenticate", request).await
    }

    async fn health(
        &self,
        request: pb::HealthRequest,
    ) -> Result<pb::HealthResponse, GuestControlHealthError> {
        self.unary("Health", request).await
    }

    async fn read_guest_file(
        &self,
        request: pb::ReadGuestFileRequest,
    ) -> Result<pb::ReadGuestFileResponse, GuestControlHealthError> {
        self.unary("ReadGuestFile", request).await
    }

    async fn usbip_import(
        &self,
        request: pb::UsbipImportRequest,
    ) -> Result<pb::UsbipImportResponse, GuestControlHealthError> {
        self.unary("UsbipImport", request).await
    }

    async fn usbip_status(
        &self,
        request: pb::UsbipStatusRequest,
    ) -> Result<pb::UsbipStatusResponse, GuestControlHealthError> {
        self.unary("UsbipStatus", request).await
    }

    async fn activate_system_start(
        &self,
        request: pb::GuestActivationStartRequest,
    ) -> Result<pb::GuestActivationStartResponse, GuestControlHealthError> {
        self.unary("ActivateSystemStart", request).await
    }

    async fn activate_system_status(
        &self,
        request: pb::GuestActivationStatusRequest,
    ) -> Result<pb::GuestActivationStatusResponse, GuestControlHealthError> {
        self.unary("ActivateSystemStatus", request).await
    }

    async fn audio_status(
        &self,
        request: pb::AudioStatusRequest,
    ) -> Result<pb::AudioStatusResponse, GuestControlHealthError> {
        self.unary("AudioStatus", request).await
    }

    async fn audio_set(
        &self,
        request: pb::AudioSetRequest,
    ) -> Result<pb::AudioSetResponse, GuestControlHealthError> {
        self.unary("AudioSet", request).await
    }
}

pub async fn probe_guest_control_health<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
) -> Result<GuestControlHealthEvidence, GuestControlHealthError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let mut hello_req = pb::HelloRequest::new();
    hello_req.metadata = MessageField::some(request_metadata(vm_id));
    hello_req.host_nonce = host_nonce.to_vec();
    hello_req.transcript_version = AUTH_TRANSCRIPT_VERSION;
    let hello = client.hello(hello_req).await?;
    if hello.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION
        || hello.guest_nonce.len() != AUTH_NONCE_LEN
        || hello.guest_boot_id.is_empty()
        || hello.guest_boot_id.len() > 128
    {
        return Err(GuestControlHealthError::Protocol);
    }
    let guest_nonce: [u8; AUTH_NONCE_LEN] = hello
        .guest_nonce
        .as_slice()
        .try_into()
        .map_err(|_| GuestControlHealthError::Protocol)?;
    let host_tag = signer
        .sign(sign_request(
            vm_id,
            GuestControlProofRole::HostProof,
            peer_cid,
            &host_nonce,
            &guest_nonce,
            &hello.guest_boot_id,
            None,
        ))?
        .tag;
    if host_tag.len() != AUTH_TAG_LEN {
        return Err(GuestControlHealthError::Signer);
    }

    let mut auth_req = pb::AuthenticateRequest::new();
    auth_req.metadata = MessageField::some(request_metadata(vm_id));
    auth_req.host_nonce = host_nonce.to_vec();
    auth_req.guest_nonce = guest_nonce.to_vec();
    auth_req.guest_boot_id = hello.guest_boot_id.clone();
    auth_req.transcript_version = AUTH_TRANSCRIPT_VERSION;
    auth_req.host_auth_tag = host_tag;
    let auth = client.authenticate(auth_req).await?;
    if auth.error.is_some() {
        return Err(GuestControlHealthError::AuthFailed);
    }
    let guest_tag = auth
        .guest_auth_tag
        .as_ref()
        .ok_or(GuestControlHealthError::AuthFailed)?;
    let capabilities_hash = auth
        .capabilities_hash
        .clone()
        .ok_or(GuestControlHealthError::Protocol)?;
    let expected_guest_tag = signer
        .sign(sign_request(
            vm_id,
            GuestControlProofRole::GuestProof,
            peer_cid,
            &host_nonce,
            &guest_nonce,
            &hello.guest_boot_id,
            Some(capabilities_hash.clone()),
        ))?
        .tag;
    if guest_tag.len() != AUTH_TAG_LEN || expected_guest_tag.len() != AUTH_TAG_LEN {
        return Err(GuestControlHealthError::AuthFailed);
    }
    if guest_tag.as_slice().ct_eq(&expected_guest_tag).unwrap_u8() != 1 {
        return Err(GuestControlHealthError::AuthFailed);
    }

    let mut health_req = pb::HealthRequest::new();
    health_req.metadata = MessageField::some(request_metadata(vm_id));
    let health = client.health(health_req).await?;
    validate_health_evidence(&health)?;
    Ok(GuestControlHealthEvidence {
        vm_id: vm_id.to_owned(),
        guest_boot_id: hello.guest_boot_id,
        protocol_version: hello.protocol_version,
        capabilities_hash,
        health,
    })
}

/// Typed outcome of an authenticated `ReadGuestFile` read. Each variant maps to
/// an operator-actionable CLI error — never a blind retry. The
/// transport/auth/protocol variants reuse the Health probe's failure taxonomy;
/// `CapabilityUnavailable` is the fail-closed result for an authenticated guest
/// that never advertised `ReadGuestFile` (an old/partial guest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestFileReadError {
    /// Handshake/transport/protocol failure (incl. unreachable, old-generation
    /// listener, auth failure) surfaced from the underlying probe.
    Probe(GuestControlHealthError),
    /// Authenticated, but the guest does not advertise `ReadGuestFile`.
    CapabilityUnavailable,
    /// The guest config working copy does not exist.
    FileNotFound,
    /// The guest config exceeds the read cap.
    FileTooLarge,
    /// The resolved path was unsafe (symlink/non-regular/`..`).
    PathUnsafe,
    /// The guest denied the read (no path wired, or permission denied).
    ReadDenied,
    /// The guest returned a malformed `ReadGuestFile` response (oversize
    /// content, unknown error kind, or content past the cap).
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestUsbipAction {
    Attach,
    Detach,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestUsbipImportResult {
    pub detached_ports: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestUsbipStatusEntry {
    pub port: u32,
    pub host: String,
    pub tcp_port: u32,
    pub bus_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestUsbipStatusResult {
    pub imports: Vec<GuestUsbipStatusEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct GuestUsbipImportCall<'a> {
    pub action: GuestUsbipAction,
    pub host: &'a str,
    pub bus_id: &'a str,
}

/// Typed outcome of authenticated guest-side USBIP import lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestUsbipImportError {
    /// Handshake/transport/protocol failure (incl. unreachable, old-generation
    /// listener, auth failure) surfaced from the underlying probe.
    Probe(GuestControlHealthError),
    /// Authenticated, but the guest does not advertise `UsbipImport`.
    CapabilityUnavailable,
    /// guestd has no usable `usbip` binary.
    UsbipUnavailable,
    /// guestd rejected the host/busid shape.
    InvalidBusId,
    /// guestd rejected the USBIP backend host address.
    InvalidHost,
    /// `usbip port`, `usbip detach`, or `usbip attach` exited unsuccessfully.
    CommandFailed,
    /// `usbip` did not complete within the strict guestd command timeout.
    CommandTimeout,
    /// `usbip port` returned output that guestd refused to parse.
    InvalidOutput,
    /// The guest returned a malformed USBIP response or wrong error kind.
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSystemActivationMode {
    Switch,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSystemActivationStart {
    pub activation_id: String,
    pub switch_script_path: String,
    pub mode: GuestSystemActivationMode,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSystemActivationStatus {
    pub state: pb::GuestActivationState,
    pub exit_code: Option<i32>,
    pub signal: Option<u32>,
    pub status_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestSystemActivationError {
    Probe(GuestControlHealthError),
    CapabilityUnavailable,
    GuestRejected(pb::GuestControlErrorKind),
    Protocol,
}

/// Typed outcome of an authenticated guest-side audio enforcement call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestAudioSetError {
    /// Handshake/transport/protocol failure.
    Probe(GuestControlHealthError),
    /// Authenticated, but the guest does not advertise `AudioSet`.
    CapabilityUnavailable,
    /// wpctl subprocess failed inside the guest (PipeWire unavailable).
    AudioPipeWireUnavailable,
    /// Level value was out of range (> 100).
    LevelOutOfRange,
    /// Unknown channel or malformed response.
    Protocol,
}

/// Per-channel state returned by a guestd audio call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAudioChannelStatus {
    pub muted: bool,
    pub level: u32,
    pub level_known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAudioStatus {
    pub microphone: GuestAudioChannelStatus,
    pub speaker: GuestAudioChannelStatus,
}

/// Authenticate to guestd and issue an `AudioStatus` RPC.
pub async fn audio_status_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
) -> Result<GuestAudioStatus, GuestAudioSetError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestAudioSetError::Probe)?;
    let advertises = evidence.health.capabilities.iter().any(|cap| {
        matches!(
            cap.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_AUDIO_STATUS)
        )
    });
    if !advertises {
        return Err(GuestAudioSetError::CapabilityUnavailable);
    }

    let mut request = pb::AudioStatusRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    let response = client
        .audio_status(request)
        .await
        .map_err(GuestAudioSetError::Probe)?;

    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_audio_error(error.kind.enum_value_or_default()));
    }
    let microphone = response
        .microphone
        .as_ref()
        .ok_or(GuestAudioSetError::Protocol)?;
    let speaker = response
        .speaker
        .as_ref()
        .ok_or(GuestAudioSetError::Protocol)?;

    Ok(GuestAudioStatus {
        microphone: GuestAudioChannelStatus {
            muted: microphone.muted,
            level: microphone.level,
            level_known: microphone.level_known,
        },
        speaker: GuestAudioChannelStatus {
            muted: speaker.muted,
            level: speaker.level,
            level_known: speaker.level_known,
        },
    })
}

/// Authenticate to the guest control endpoint and issue an `AudioSet` RPC.
///
/// The `AudioSet` capability MUST be advertised; an authenticated guest that
/// never advertised it fails closed (`CapabilityUnavailable`). This prevents
/// a silent no-op on an older guest generation that predates audio support.
#[derive(Debug, Clone, Copy)]
pub struct GuestAudioSetRequest {
    pub channel: pb::AudioChannel,
    pub kind: pb::AudioSetKind,
    pub grant_on: bool,
    pub level: u32,
}

pub async fn audio_set_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
    audio_set: GuestAudioSetRequest,
) -> Result<GuestAudioChannelStatus, GuestAudioSetError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestAudioSetError::Probe)?;
    let advertises = evidence.health.capabilities.iter().any(|cap| {
        matches!(
            cap.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_AUDIO_SET)
        )
    });
    if !advertises {
        return Err(GuestAudioSetError::CapabilityUnavailable);
    }

    let mut request = pb::AudioSetRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.channel = protobuf::EnumOrUnknown::new(audio_set.channel);
    request.kind = protobuf::EnumOrUnknown::new(audio_set.kind);
    request.grant_on = audio_set.grant_on;
    request.level = audio_set.level;

    let response = client
        .audio_set(request)
        .await
        .map_err(GuestAudioSetError::Probe)?;

    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_audio_error(error.kind.enum_value_or_default()));
    }

    let state = response
        .state
        .as_ref()
        .ok_or(GuestAudioSetError::Protocol)?;
    Ok(GuestAudioChannelStatus {
        muted: state.muted,
        level: state.level,
        level_known: state.level_known,
    })
}

fn map_guest_audio_error(kind: pb::GuestControlErrorKind) -> GuestAudioSetError {
    use pb::GuestControlErrorKind as K;
    match kind {
        K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE => {
            GuestAudioSetError::AudioPipeWireUnavailable
        }
        K::GUEST_CONTROL_ERROR_KIND_AUDIO_LEVEL_OUT_OF_RANGE => GuestAudioSetError::LevelOutOfRange,
        K::GUEST_CONTROL_ERROR_KIND_AUDIO_CHANNEL_UNKNOWN => GuestAudioSetError::Protocol,
        _ => GuestAudioSetError::Protocol,
    }
}

/// Authenticate to the guest control endpoint (reusing the W11 Health-probe
/// handshake) and read the editable guest config working copy via the typed
/// `ReadGuestFile { GuestConfig }` RPC on the SAME authenticated connection.
///
/// The negotiated `ReadGuestFile` capability is REQUIRED — an
/// authenticated guest that never advertised it fails closed
/// (`CapabilityUnavailable`) instead of being probed for a config file.
///
/// The returned bytes are the integrity ground truth; the guest's
/// self-reported `size_bytes`/`sha256` are ignored here and recomputed by the
/// host. This function returns ONLY the raw bytes (or a typed error) and never
/// leaks them into the error path.
pub async fn read_guest_config_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
) -> Result<Vec<u8>, GuestFileReadError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestFileReadError::Probe)?;
    let advertises_read = evidence.health.capabilities.iter().any(|capability| {
        matches!(
            capability.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_READ_GUEST_FILE)
        )
    });
    if !advertises_read {
        return Err(GuestFileReadError::CapabilityUnavailable);
    }

    let mut request = pb::ReadGuestFileRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.file_id = protobuf::EnumOrUnknown::new(pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG);
    let response = client
        .read_guest_file(request)
        .await
        .map_err(GuestFileReadError::Probe)?;

    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_file_error(error.kind.enum_value_or_default()));
    }
    // Defense in depth: a well-behaved guest never returns content past the cap,
    // but the host re-enforces the bound on RECEIVED bytes and never trusts the
    // guest-reported size/hash.
    if response.content.len() as u64 > READ_GUEST_FILE_MAX_BYTES {
        return Err(GuestFileReadError::Protocol);
    }
    Ok(response.content)
}

pub async fn usbip_import_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
    call: GuestUsbipImportCall<'_>,
) -> Result<GuestUsbipImportResult, GuestUsbipImportError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestUsbipImportError::Probe)?;
    let advertises_usbip = evidence.health.capabilities.iter().any(|capability| {
        matches!(
            capability.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_USBIP_IMPORT)
        )
    });
    if !advertises_usbip {
        return Err(GuestUsbipImportError::CapabilityUnavailable);
    }

    let wire_action = match call.action {
        GuestUsbipAction::Attach => pb::UsbipImportAction::USBIP_IMPORT_ACTION_ATTACH,
        GuestUsbipAction::Detach => pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH,
    };
    let mut request = pb::UsbipImportRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.action = protobuf::EnumOrUnknown::new(wire_action);
    request.host = call.host.to_owned();
    request.bus_id = call.bus_id.to_owned();
    let response = client
        .usbip_import(request)
        .await
        .map_err(GuestUsbipImportError::Probe)?;

    if response.action.enum_value_or_default() != wire_action || response.bus_id != call.bus_id {
        return Err(GuestUsbipImportError::Protocol);
    }
    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_usbip_error(error.kind.enum_value_or_default()));
    }
    Ok(GuestUsbipImportResult {
        detached_ports: response.detached_ports,
    })
}

pub async fn usbip_status_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
    host: Option<&str>,
    bus_id: Option<&str>,
) -> Result<GuestUsbipStatusResult, GuestUsbipImportError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestUsbipImportError::Probe)?;
    let advertises_usbip_status = evidence.health.capabilities.iter().any(|capability| {
        matches!(
            capability.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_USBIP_STATUS)
        )
    });
    if !advertises_usbip_status {
        return Err(GuestUsbipImportError::CapabilityUnavailable);
    }

    let mut request = pb::UsbipStatusRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.host = host.map(str::to_owned);
    request.bus_id = bus_id.map(str::to_owned);
    let response = client
        .usbip_status(request)
        .await
        .map_err(GuestUsbipImportError::Probe)?;

    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_usbip_error(error.kind.enum_value_or_default()));
    }
    let mut imports = Vec::with_capacity(response.imports.len());
    for entry in response.imports {
        if entry.port > u16::MAX as u32
            || entry.tcp_port == 0
            || entry.tcp_port > u16::MAX as u32
            || entry.host.parse::<std::net::IpAddr>().is_err()
            || d2b_contracts::usbip::validate_bus_id(&entry.bus_id).is_err()
        {
            return Err(GuestUsbipImportError::Protocol);
        }
        imports.push(GuestUsbipStatusEntry {
            port: entry.port,
            host: entry.host,
            tcp_port: entry.tcp_port,
            bus_id: entry.bus_id,
        });
    }
    Ok(GuestUsbipStatusResult { imports })
}

pub async fn activate_system_start_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
    start: &GuestSystemActivationStart,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestSystemActivationError::Probe)?;
    let advertises_activation = evidence.health.capabilities.iter().any(|capability| {
        matches!(
            capability.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_SYSTEM_ACTIVATION)
        )
    });
    if !advertises_activation {
        return Err(GuestSystemActivationError::CapabilityUnavailable);
    }

    let mut request = pb::GuestActivationStartRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.activation_id = start.activation_id.clone();
    request.switch_script_path = start.switch_script_path.clone();
    request.timeout_ms = start.timeout_ms;
    request.mode = protobuf::EnumOrUnknown::new(match start.mode {
        GuestSystemActivationMode::Switch => pb::GuestActivationMode::GUEST_ACTIVATION_MODE_SWITCH,
        GuestSystemActivationMode::Test => pb::GuestActivationMode::GUEST_ACTIVATION_MODE_TEST,
    });
    let response = client
        .activate_system_start(request)
        .await
        .map_err(GuestSystemActivationError::Probe)?;
    if response.activation_id != start.activation_id {
        return Err(GuestSystemActivationError::Protocol);
    }
    if let Some(error) = response.error.as_ref() {
        return Err(GuestSystemActivationError::GuestRejected(
            error.kind.enum_value_or_default(),
        ));
    }
    let state = response
        .state
        .enum_value()
        .map_err(|_| GuestSystemActivationError::Protocol)?;
    Ok(GuestSystemActivationStatus {
        state,
        exit_code: None,
        signal: None,
        status_code: None,
    })
}

pub async fn activate_system_status_authenticated<C, S>(
    vm_id: &str,
    peer_cid: Option<u32>,
    host_nonce: [u8; AUTH_NONCE_LEN],
    client: &C,
    signer: &S,
    activation_id: &str,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError>
where
    C: GuestControlRpc + Sync,
    S: GuestControlSigner + Sync,
{
    let evidence = probe_guest_control_health(vm_id, peer_cid, host_nonce, client, signer)
        .await
        .map_err(GuestSystemActivationError::Probe)?;
    let advertises_activation = evidence.health.capabilities.iter().any(|capability| {
        matches!(
            capability.enum_value(),
            Ok(pb::GuestCapability::GUEST_CAPABILITY_SYSTEM_ACTIVATION)
        )
    });
    if !advertises_activation {
        return Err(GuestSystemActivationError::CapabilityUnavailable);
    }

    let mut request = pb::GuestActivationStatusRequest::new();
    request.metadata = MessageField::some(request_metadata(vm_id));
    request.activation_id = activation_id.to_owned();
    let response = client
        .activate_system_status(request)
        .await
        .map_err(GuestSystemActivationError::Probe)?;
    if response.activation_id != activation_id {
        return Err(GuestSystemActivationError::Protocol);
    }
    if let Some(error) = response.error.as_ref() {
        return Err(GuestSystemActivationError::GuestRejected(
            error.kind.enum_value_or_default(),
        ));
    }
    let state = response
        .state
        .enum_value()
        .map_err(|_| GuestSystemActivationError::Protocol)?;
    Ok(GuestSystemActivationStatus {
        state,
        exit_code: response.exit_code,
        signal: response.signal,
        status_code: response.status_code,
    })
}

/// Exhaustive host-side mapping of a guest `ReadGuestFile` error kind to a typed
/// read error (no default `Retry`). Non-file kinds collapse to
/// `Protocol` because the guest must not return them on this RPC.
fn map_guest_file_error(kind: pb::GuestControlErrorKind) -> GuestFileReadError {
    use pb::GuestControlErrorKind as K;
    match kind {
        K::GUEST_CONTROL_ERROR_KIND_FILE_NOT_FOUND => GuestFileReadError::FileNotFound,
        K::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE => GuestFileReadError::FileTooLarge,
        K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE => GuestFileReadError::PathUnsafe,
        K::GUEST_CONTROL_ERROR_KIND_READ_DENIED => GuestFileReadError::ReadDenied,
        _ => GuestFileReadError::Protocol,
    }
}

fn map_guest_usbip_error(kind: pb::GuestControlErrorKind) -> GuestUsbipImportError {
    use pb::GuestControlErrorKind as K;
    match kind {
        K::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE => GuestUsbipImportError::UsbipUnavailable,
        K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_BUS_ID => GuestUsbipImportError::InvalidBusId,
        K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST => GuestUsbipImportError::InvalidHost,
        K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED => GuestUsbipImportError::CommandFailed,
        K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT => GuestUsbipImportError::CommandTimeout,
        K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT => GuestUsbipImportError::InvalidOutput,
        _ => GuestUsbipImportError::Protocol,
    }
}

/// Map an authenticated guest-control Health probe outcome to a framework
/// readiness decision (readiness DAG migration).
///
/// Fails CLOSED: a node is ready only when the daemon completed the full
/// authenticated Hello + token challenge-response + Health handshake AND the
/// guest reported a `Healthy` or `Degraded` state. An old-generation /
/// unreachable / auth-failed / timed-out / protocol-violating guest (every
/// `Err`) is never ready. This is the deliberate contrast with
/// `ReadinessPredicate::ComponentSpecific`, which reports ready unconditionally
/// and would fail OPEN.
pub fn guest_control_health_ready(
    outcome: &Result<GuestControlHealthEvidence, GuestControlHealthError>,
) -> bool {
    match outcome {
        Ok(evidence) => matches!(
            evidence.health.state.enum_value(),
            Ok(pb::HealthState::HEALTH_STATE_HEALTHY) | Ok(pb::HealthState::HEALTH_STATE_DEGRADED)
        ),
        Err(_) => false,
    }
}

fn validate_health_evidence(health: &pb::HealthResponse) -> Result<(), GuestControlHealthError> {
    let origin = health
        .origin
        .enum_value()
        .map_err(|_| GuestControlHealthError::Protocol)?;
    let state = health
        .state
        .enum_value()
        .map_err(|_| GuestControlHealthError::Protocol)?;
    let reason = health
        .reason
        .enum_value()
        .map_err(|_| GuestControlHealthError::Protocol)?;
    let remediation = health
        .remediation
        .enum_value()
        .map_err(|_| GuestControlHealthError::Protocol)?;
    if origin != pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED
        || health.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION
    {
        return Err(GuestControlHealthError::Protocol);
    }
    if health.capabilities.len() > 32
        || health.degraded_subsystems.len() > 16
        || health.capabilities.iter().any(|capability| {
            !matches!(
                capability.enum_value(),
                Ok(value) if value != pb::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED
            )
        })
        || health.degraded_subsystems.iter().any(|subsystem| {
            !matches!(
                subsystem.enum_value(),
                Ok(value) if value != pb::GuestSubsystem::GUEST_SUBSYSTEM_UNSPECIFIED
            )
        })
    {
        return Err(GuestControlHealthError::Protocol);
    }
    match state {
        pb::HealthState::HEALTH_STATE_HEALTHY => {
            if reason != pb::HealthReason::HEALTH_REASON_NONE
                || remediation != pb::HealthRemediation::HEALTH_REMEDIATION_NONE
                || !health.degraded_subsystems.is_empty()
            {
                return Err(GuestControlHealthError::Protocol);
            }
        }
        pb::HealthState::HEALTH_STATE_DEGRADED => {
            let valid_reason = matches!(
                reason,
                pb::HealthReason::HEALTH_REASON_EXEC_SUBSYSTEM_UNAVAILABLE
                    | pb::HealthReason::HEALTH_REASON_LOG_STORAGE_UNAVAILABLE
                    | pb::HealthReason::HEALTH_REASON_QUOTA_EXCEEDED
                    | pb::HealthReason::HEALTH_REASON_RATE_LIMITED
                    | pb::HealthReason::HEALTH_REASON_INTERNAL_HEALTH_CHECK_FAILED
            );
            let valid_remediation = matches!(
                remediation,
                pb::HealthRemediation::HEALTH_REMEDIATION_RETRY
                    | pb::HealthRemediation::HEALTH_REMEDIATION_REDUCE_LOAD
                    | pb::HealthRemediation::HEALTH_REMEDIATION_INSPECT_GUEST_LOGS
                    | pb::HealthRemediation::HEALTH_REMEDIATION_RESTART_VM
            );
            if !valid_reason || !valid_remediation || health.degraded_subsystems.is_empty() {
                return Err(GuestControlHealthError::Protocol);
            }
        }
        _ => return Err(GuestControlHealthError::Protocol),
    }
    Ok(())
}

fn request_metadata(vm_id: &str) -> pb::RequestMetadata {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = vm_id.to_owned();
    metadata.request_id = "guest-health-probe".to_owned();
    metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
    metadata
}

fn sign_request(
    vm_id: &str,
    role: GuestControlProofRole,
    peer_cid: Option<u32>,
    host_nonce: &[u8; AUTH_NONCE_LEN],
    guest_nonce: &[u8; AUTH_NONCE_LEN],
    guest_boot_id: &str,
    capabilities_hash: Option<String>,
) -> GuestControlSignRequest {
    GuestControlSignRequest {
        vm_id: d2b_contracts::types::VmId::new(vm_id),
        role,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        direction: GuestControlDirection::HostToGuest,
        purpose: GuestControlAuthPurpose::GuestControlAuthV1,
        guest_control_port: GUEST_CONTROL_AUTH_PORT,
        peer_cid,
        host_nonce: host_nonce.to_vec(),
        guest_nonce: guest_nonce.to_vec(),
        guest_boot_id: GuestBootIdWire::new(guest_boot_id),
        capabilities_hash,
        tracing_span_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSigner;

    impl GuestControlSigner for FakeSigner {
        fn sign(
            &self,
            request: GuestControlSignRequest,
        ) -> Result<GuestControlSignResponse, GuestControlHealthError> {
            request
                .validate_shape()
                .map_err(|_| GuestControlHealthError::Signer)?;
            let fill = match request.role {
                GuestControlProofRole::HostProof => 0x55,
                GuestControlProofRole::GuestProof => 0x77,
            };
            Ok(GuestControlSignResponse {
                tag: vec![fill; AUTH_TAG_LEN],
            })
        }
    }

    #[derive(Default)]
    struct FakeClient {
        bad_guest_tag: bool,
        overlong_boot_id: bool,
        invalid_health: bool,
        advertise_read_cap: bool,
        advertise_usbip_cap: bool,
        advertise_usbip_status_cap: bool,
        read_error: Option<pb::GuestControlErrorKind>,
        read_content: Vec<u8>,
        usbip_error: Option<pb::GuestControlErrorKind>,
        usbip_wrong_echo: bool,
        usbip_status_invalid_entry: bool,
    }

    #[async_trait]
    impl GuestControlRpc for FakeClient {
        async fn hello(
            &self,
            _request: pb::HelloRequest,
        ) -> Result<pb::HelloResponse, GuestControlHealthError> {
            let mut response = pb::HelloResponse::new();
            response.guest_nonce = vec![0x22; AUTH_NONCE_LEN];
            response.guest_boot_id = if self.overlong_boot_id {
                "b".repeat(129)
            } else {
                "boot-1".to_owned()
            };
            response.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
            Ok(response)
        }

        async fn authenticate(
            &self,
            _request: pb::AuthenticateRequest,
        ) -> Result<pb::AuthenticateResponse, GuestControlHealthError> {
            let mut response = pb::AuthenticateResponse::new();
            response.guest_auth_tag = Some(vec![
                if self.bad_guest_tag { 0x99 } else { 0x77 };
                AUTH_TAG_LEN
            ]);
            response.capabilities_hash = Some("caps-sha256".to_owned());
            Ok(response)
        }

        async fn health(
            &self,
            _request: pb::HealthRequest,
        ) -> Result<pb::HealthResponse, GuestControlHealthError> {
            let mut health = pb::HealthResponse::new();
            health.origin = protobuf::EnumOrUnknown::new(if self.invalid_health {
                pb::HealthOrigin::HEALTH_ORIGIN_HOST_SYNTHESIZED
            } else {
                pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED
            });
            health.state = protobuf::EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
            health.reason = protobuf::EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
            health.remediation =
                protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
            health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
            if self.advertise_read_cap {
                health.capabilities.push(protobuf::EnumOrUnknown::new(
                    pb::GuestCapability::GUEST_CAPABILITY_READ_GUEST_FILE,
                ));
            }
            if self.advertise_usbip_cap {
                health.capabilities.push(protobuf::EnumOrUnknown::new(
                    pb::GuestCapability::GUEST_CAPABILITY_USBIP_IMPORT,
                ));
            }
            if self.advertise_usbip_status_cap {
                health.capabilities.push(protobuf::EnumOrUnknown::new(
                    pb::GuestCapability::GUEST_CAPABILITY_USBIP_STATUS,
                ));
            }
            Ok(health)
        }

        async fn read_guest_file(
            &self,
            _request: pb::ReadGuestFileRequest,
        ) -> Result<pb::ReadGuestFileResponse, GuestControlHealthError> {
            let mut response = pb::ReadGuestFileResponse::new();
            response.file_id =
                protobuf::EnumOrUnknown::new(pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG);
            if let Some(kind) = self.read_error {
                let mut error = pb::GuestControlError::new();
                error.kind = protobuf::EnumOrUnknown::new(kind);
                response.error = MessageField::some(error);
            } else {
                response.content = self.read_content.clone();
                response.size_bytes = self.read_content.len() as u64;
            }
            Ok(response)
        }

        async fn usbip_import(
            &self,
            request: pb::UsbipImportRequest,
        ) -> Result<pb::UsbipImportResponse, GuestControlHealthError> {
            let mut response = pb::UsbipImportResponse::new();
            response.action = request.action;
            response.bus_id = if self.usbip_wrong_echo {
                "9-9".to_owned()
            } else {
                request.bus_id
            };
            response.detached_ports = 1;
            if let Some(kind) = self.usbip_error {
                let mut error = pb::GuestControlError::new();
                error.kind = protobuf::EnumOrUnknown::new(kind);
                response.error = MessageField::some(error);
            }
            Ok(response)
        }

        async fn usbip_status(
            &self,
            _request: pb::UsbipStatusRequest,
        ) -> Result<pb::UsbipStatusResponse, GuestControlHealthError> {
            let mut response = pb::UsbipStatusResponse::new();
            if let Some(kind) = self.usbip_error {
                let mut error = pb::GuestControlError::new();
                error.kind = protobuf::EnumOrUnknown::new(kind);
                response.error = MessageField::some(error);
                return Ok(response);
            }
            let mut entry = pb::UsbipStatusEntry::new();
            entry.port = 1;
            entry.host = if self.usbip_status_invalid_entry {
                "not-an-ip".to_owned()
            } else {
                "192.0.2.1".to_owned()
            };
            entry.tcp_port = 3240;
            entry.bus_id = "1-2".to_owned();
            response.imports.push(entry);
            Ok(response)
        }
    }

    #[tokio::test]
    async fn probe_verifies_guest_tag_before_returning_health() {
        let evidence = probe_guest_control_health(
            "corp-vm",
            Some(2),
            [0x11; AUTH_NONCE_LEN],
            &FakeClient {
                bad_guest_tag: false,
                overlong_boot_id: false,
                invalid_health: false,
                ..Default::default()
            },
            &FakeSigner,
        )
        .await
        .expect("probe succeeds");
        assert_eq!(evidence.vm_id, "corp-vm");
        assert_eq!(evidence.guest_boot_id, "boot-1");

        assert!(matches!(
            probe_guest_control_health(
                "corp-vm",
                Some(2),
                [0x11; AUTH_NONCE_LEN],
                &FakeClient {
                    bad_guest_tag: true,
                    overlong_boot_id: false,
                    invalid_health: false,
                    ..Default::default()
                },
                &FakeSigner,
            )
            .await,
            Err(GuestControlHealthError::AuthFailed)
        ));

        assert!(matches!(
            probe_guest_control_health(
                "corp-vm",
                Some(2),
                [0x11; AUTH_NONCE_LEN],
                &FakeClient {
                    bad_guest_tag: false,
                    overlong_boot_id: true,
                    invalid_health: false,
                    ..Default::default()
                },
                &FakeSigner,
            )
            .await,
            Err(GuestControlHealthError::Protocol)
        ));

        assert!(matches!(
            probe_guest_control_health(
                "corp-vm",
                Some(2),
                [0x11; AUTH_NONCE_LEN],
                &FakeClient {
                    bad_guest_tag: false,
                    overlong_boot_id: false,
                    invalid_health: true,
                    ..Default::default()
                },
                &FakeSigner,
            )
            .await,
            Err(GuestControlHealthError::Protocol)
        ));
    }

    async fn read_config(client: &FakeClient) -> Result<Vec<u8>, GuestFileReadError> {
        read_guest_config_authenticated(
            "corp-vm",
            Some(2),
            [0x11; AUTH_NONCE_LEN],
            client,
            &FakeSigner,
        )
        .await
    }

    #[tokio::test]
    async fn read_guest_config_returns_received_bytes_when_cap_advertised() {
        let bytes = read_config(&FakeClient {
            advertise_read_cap: true,
            read_content: b"hostname = corp-vm\n".to_vec(),
            ..Default::default()
        })
        .await
        .expect("read succeeds");
        assert_eq!(bytes, b"hostname = corp-vm\n");
    }

    #[tokio::test]
    async fn read_guest_config_fails_closed_without_capability() {
        // D15: an authenticated guest that never advertised ReadGuestFile is an
        // old/partial guest; the read fails closed rather than being attempted.
        assert_eq!(
            read_config(&FakeClient {
                advertise_read_cap: false,
                read_content: b"should-not-be-read".to_vec(),
                ..Default::default()
            })
            .await,
            Err(GuestFileReadError::CapabilityUnavailable)
        );
    }

    #[tokio::test]
    async fn read_guest_config_auth_failure_never_reaches_read() {
        assert_eq!(
            read_config(&FakeClient {
                bad_guest_tag: true,
                advertise_read_cap: true,
                read_content: b"unreachable".to_vec(),
                ..Default::default()
            })
            .await,
            Err(GuestFileReadError::Probe(
                GuestControlHealthError::AuthFailed
            ))
        );
    }

    #[tokio::test]
    async fn read_guest_config_maps_each_file_error_kind() {
        for (kind, expected) in [
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_FILE_NOT_FOUND,
                GuestFileReadError::FileNotFound,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE,
                GuestFileReadError::FileTooLarge,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE,
                GuestFileReadError::PathUnsafe,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_READ_DENIED,
                GuestFileReadError::ReadDenied,
            ),
            (
                // A non-file kind on this RPC is a protocol violation, not a
                // blind retry (D12).
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
                GuestFileReadError::Protocol,
            ),
        ] {
            let result = read_config(&FakeClient {
                advertise_read_cap: true,
                read_error: Some(kind),
                ..Default::default()
            })
            .await;
            assert_eq!(result, Err(expected), "kind {kind:?}");
        }
    }

    async fn usbip_import(
        client: &FakeClient,
    ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
        usbip_import_authenticated(
            "corp-vm",
            Some(2),
            [0x11; AUTH_NONCE_LEN],
            client,
            &FakeSigner,
            GuestUsbipImportCall {
                action: GuestUsbipAction::Attach,
                host: "192.0.2.1",
                bus_id: "1-2",
            },
        )
        .await
    }

    async fn usbip_status(
        client: &FakeClient,
    ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
        usbip_status_authenticated(
            "corp-vm",
            Some(2),
            [0x11; AUTH_NONCE_LEN],
            client,
            &FakeSigner,
            Some("192.0.2.1"),
            Some("1-2"),
        )
        .await
    }

    #[tokio::test]
    async fn usbip_import_requires_advertised_capability() {
        assert_eq!(
            usbip_import(&FakeClient {
                advertise_usbip_cap: false,
                ..Default::default()
            })
            .await,
            Err(GuestUsbipImportError::CapabilityUnavailable)
        );
    }

    #[tokio::test]
    async fn usbip_import_returns_detached_port_count() {
        let result = usbip_import(&FakeClient {
            advertise_usbip_cap: true,
            ..Default::default()
        })
        .await
        .expect("usbip import succeeds");
        assert_eq!(result.detached_ports, 1);
    }

    #[tokio::test]
    async fn usbip_import_maps_closed_guest_errors() {
        for (kind, expected) in [
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE,
                GuestUsbipImportError::UsbipUnavailable,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_BUS_ID,
                GuestUsbipImportError::InvalidBusId,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST,
                GuestUsbipImportError::InvalidHost,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED,
                GuestUsbipImportError::CommandFailed,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT,
                GuestUsbipImportError::CommandTimeout,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT,
                GuestUsbipImportError::InvalidOutput,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
                GuestUsbipImportError::Protocol,
            ),
        ] {
            let result = usbip_import(&FakeClient {
                advertise_usbip_cap: true,
                usbip_error: Some(kind),
                ..Default::default()
            })
            .await;
            assert_eq!(result, Err(expected), "kind {kind:?}");
        }
    }

    #[tokio::test]
    async fn usbip_import_rejects_mismatched_echo() {
        assert_eq!(
            usbip_import(&FakeClient {
                advertise_usbip_cap: true,
                usbip_wrong_echo: true,
                ..Default::default()
            })
            .await,
            Err(GuestUsbipImportError::Protocol)
        );
    }

    #[tokio::test]
    async fn usbip_status_requires_advertised_capability() {
        assert_eq!(
            usbip_status(&FakeClient {
                advertise_usbip_cap: true,
                advertise_usbip_status_cap: false,
                ..Default::default()
            })
            .await,
            Err(GuestUsbipImportError::CapabilityUnavailable)
        );
    }

    #[tokio::test]
    async fn usbip_status_returns_sanitized_import_entries() {
        let result = usbip_status(&FakeClient {
            advertise_usbip_status_cap: true,
            ..Default::default()
        })
        .await
        .expect("usbip status succeeds");
        assert_eq!(
            result.imports,
            vec![GuestUsbipStatusEntry {
                port: 1,
                host: "192.0.2.1".to_owned(),
                tcp_port: 3240,
                bus_id: "1-2".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn usbip_status_maps_closed_guest_errors_and_invalid_entries() {
        for (kind, expected) in [
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT,
                GuestUsbipImportError::CommandTimeout,
            ),
            (
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT,
                GuestUsbipImportError::InvalidOutput,
            ),
        ] {
            let result = usbip_status(&FakeClient {
                advertise_usbip_status_cap: true,
                usbip_error: Some(kind),
                ..Default::default()
            })
            .await;
            assert_eq!(result, Err(expected), "kind {kind:?}");
        }

        assert_eq!(
            usbip_status(&FakeClient {
                advertise_usbip_status_cap: true,
                usbip_status_invalid_entry: true,
                ..Default::default()
            })
            .await,
            Err(GuestUsbipImportError::Protocol)
        );
    }

    fn evidence_with_state(state: pb::HealthState) -> GuestControlHealthEvidence {
        let mut health = pb::HealthResponse::new();
        health.state = protobuf::EnumOrUnknown::new(state);
        GuestControlHealthEvidence {
            vm_id: "corp-vm".to_owned(),
            guest_boot_id: "boot-1".to_owned(),
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities_hash: "caps-sha256".to_owned(),
            health,
        }
    }

    #[test]
    fn guest_control_health_ready_is_fail_closed() {
        // A successfully authenticated guest reporting healthy or degraded is
        // ready (D5).
        assert!(guest_control_health_ready(&Ok(evidence_with_state(
            pb::HealthState::HEALTH_STATE_HEALTHY
        ))));
        assert!(guest_control_health_ready(&Ok(evidence_with_state(
            pb::HealthState::HEALTH_STATE_DEGRADED
        ))));
        // Any other reported state is never ready (defense in depth — the probe
        // already rejects these, but the decision is fail-closed regardless).
        assert!(!guest_control_health_ready(&Ok(evidence_with_state(
            pb::HealthState::HEALTH_STATE_UNAVAILABLE_OLD_GENERATION
        ))));
        assert!(!guest_control_health_ready(&Ok(evidence_with_state(
            pb::HealthState::HEALTH_STATE_LISTENER_ABSENT
        ))));
        assert!(!guest_control_health_ready(&Ok(evidence_with_state(
            pb::HealthState::HEALTH_STATE_UNSPECIFIED
        ))));
        // Every probe failure — old generation, unreachable, auth failure,
        // timeout, protocol violation, stale session — fails closed.
        for error in [
            GuestControlHealthError::TransportIo,
            GuestControlHealthError::Ttrpc,
            GuestControlHealthError::Signer,
            GuestControlHealthError::Protocol,
            GuestControlHealthError::AuthFailed,
            GuestControlHealthError::StaleSession,
            GuestControlHealthError::Timeout,
        ] {
            assert!(
                !guest_control_health_ready(&Err(error.clone())),
                "error {error:?} must fail closed"
            );
        }
    }

    #[test]
    fn ttrpc_deadline_errors_map_to_timeout() {
        let deadline =
            ttrpc::Error::RpcStatus(ttrpc::get_status(ttrpc::Code::DEADLINE_EXCEEDED, "timeout"));
        assert_eq!(
            map_ttrpc_request_error(deadline),
            GuestControlHealthError::Timeout
        );
        assert_eq!(
            map_ttrpc_request_error(ttrpc::Error::Others(
                "Receive packet timeout elapsed".to_owned()
            )),
            GuestControlHealthError::Timeout
        );
        assert_eq!(
            map_ttrpc_request_error(ttrpc::Error::Others("stream reset".to_owned())),
            GuestControlHealthError::Ttrpc
        );
    }

    #[test]
    fn attempt_budget_caps_each_op_and_expires_at_deadline() {
        // A budget with plenty of headroom yields the per-op cap, never
        // the full remaining deadline.
        let budget = AttemptBudget::from_now(Duration::from_secs(30), Duration::from_secs(3));
        let next = budget.next().expect("budget not yet expired");
        assert!(next <= Duration::from_secs(3));
        assert!(!budget.is_expired());

        // A zero-span budget is immediately expired: next() is None so the
        // caller surfaces a Timeout instead of issuing an unbounded op.
        let expired = AttemptBudget::from_now(Duration::ZERO, Duration::from_secs(3));
        assert!(expired.next().is_none());
        assert!(expired.is_expired());
    }

    #[test]
    fn attempt_budget_remaining_below_cap_is_used() {
        // When the remaining deadline is smaller than the cap, the op gets
        // the (smaller) remaining budget, so connect + ttRPC + sign share
        // one absolute deadline rather than each getting the full cap.
        let budget = AttemptBudget::from_now(Duration::from_millis(40), Duration::from_secs(3));
        let next = budget.next().expect("budget not yet expired");
        assert!(next <= Duration::from_millis(40));
    }
}
