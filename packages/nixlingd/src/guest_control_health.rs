//! Host-side authenticated guest-control Health probe.
//!
//! W11 stores authenticated health evidence only. It does not replace VM
//! lifecycle readiness and it does not expose exec.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nixling_ipc::broker_wire::{
    GuestBootIdWire, GuestControlAuthPurpose, GuestControlDirection, GuestControlProofRole,
    GuestControlSignRequest, GuestControlSignResponse,
};
use nixling_ipc::guest_auth::{
    AUTH_NONCE_LEN, AUTH_TAG_LEN, AUTH_TRANSCRIPT_VERSION, GUEST_CONTROL_AUTH_PORT,
};
use nixling_ipc::guest_proto as pb;
use nixling_ipc::guest_wire::{GUEST_CONTROL_PROTOCOL_VERSION, READ_GUEST_FILE_MAX_BYTES};
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
        let timeout_nano = timeout.as_nanos().min(i64::MAX as u128) as i64;
        let mut payload = Vec::new();
        request
            .write_to_vec(&mut payload)
            .map_err(|_| GuestControlHealthError::Protocol)?;
        let response = self
            .client
            .request(ttrpc::Request {
                service: "nixling.guest.v1.GuestControl".to_owned(),
                method: method.to_owned(),
                timeout_nano,
                metadata: ttrpc::context::to_pb(HashMap::new()),
                payload,
                ..Default::default()
            })
            .await
            .map_err(|_| GuestControlHealthError::Ttrpc)?;
        Resp::parse_from_bytes(&response.payload).map_err(|_| GuestControlHealthError::Protocol)
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
/// an operator-actionable CLI error (Decision 12) — never a blind retry. The
/// transport/auth/protocol variants reuse the Health probe's failure taxonomy;
/// `CapabilityUnavailable` is the fail-closed result for an authenticated guest
/// that never advertised `ReadGuestFile` (Decision 15 — an old/partial guest).
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

/// Authenticate to the guest control endpoint (reusing the W11 Health-probe
/// handshake) and read the editable guest config working copy via the typed
/// `ReadGuestFile { GuestConfig }` RPC on the SAME authenticated connection.
///
/// Decision 15: the negotiated `ReadGuestFile` capability is REQUIRED — an
/// authenticated guest that never advertised it fails closed
/// (`CapabilityUnavailable`) instead of being probed for a config file.
///
/// Decision 4: the returned bytes are the integrity ground truth; the guest's
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
    request.file_id =
        protobuf::EnumOrUnknown::new(pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG);
    let response = client
        .read_guest_file(request)
        .await
        .map_err(GuestFileReadError::Probe)?;

    if let Some(error) = response.error.as_ref() {
        return Err(map_guest_file_error(error.kind.enum_value_or_default()));
    }
    // Defense in depth: a well-behaved guest never returns content past the cap,
    // but the host re-enforces the bound on RECEIVED bytes and never trusts the
    // guest-reported size/hash (Decision 4).
    if response.content.len() as u64 > READ_GUEST_FILE_MAX_BYTES {
        return Err(GuestFileReadError::Protocol);
    }
    Ok(response.content)
}

/// Exhaustive host-side mapping of a guest `ReadGuestFile` error kind to a typed
/// read error (Decision 12 — no default `Retry`). Non-file kinds collapse to
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

/// Map an authenticated guest-control Health probe outcome to a framework
/// readiness decision (W15 readiness DAG migration, Decision 5).
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
            Ok(pb::HealthState::HEALTH_STATE_HEALTHY)
                | Ok(pb::HealthState::HEALTH_STATE_DEGRADED)
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
        vm_id: nixling_ipc::types::VmId::new(vm_id),
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
        read_error: Option<pb::GuestControlErrorKind>,
        read_content: Vec<u8>,
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
        read_guest_config_authenticated("corp-vm", Some(2), [0x11; AUTH_NONCE_LEN], client, &FakeSigner)
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
            Err(GuestFileReadError::Probe(GuestControlHealthError::AuthFailed))
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
