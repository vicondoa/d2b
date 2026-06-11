//! Host-side authenticated guest-control Health probe.
//!
//! W11 stores authenticated health evidence only. It does not replace VM
//! lifecycle readiness and it does not expose exec.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::Duration;

use async_trait::async_trait;
use nixling_ipc::broker_wire::{
    GuestBootIdWire, GuestControlAuthPurpose, GuestControlDirection, GuestControlProofRole,
    GuestControlSignRequest, GuestControlSignResponse,
};
use nixling_ipc::guest_auth::{
    AUTH_NONCE_LEN, AUTH_TAG_LEN, AUTH_TRANSCRIPT_VERSION, GUEST_CONTROL_AUTH_PORT,
};
use nixling_ipc::guest_proto as pb;
use nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
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
    async fn hello(&self, request: pb::HelloRequest) -> Result<pb::HelloResponse, GuestControlHealthError>;
    async fn authenticate(
        &self,
        request: pb::AuthenticateRequest,
    ) -> Result<pb::AuthenticateResponse, GuestControlHealthError>;
    async fn health(&self, request: pb::HealthRequest) -> Result<pb::HealthResponse, GuestControlHealthError>;
}

pub trait GuestControlSigner {
    fn sign(&self, request: GuestControlSignRequest) -> Result<GuestControlSignResponse, GuestControlHealthError>;
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
    timeout_nano: i64,
}

impl TtrpcGuestControlClient {
    pub fn new(socket: ttrpc::r#async::transport::Socket, timeout: Duration) -> Self {
        Self {
            client: ttrpc::r#async::Client::new(socket),
            timeout_nano: timeout.as_nanos().min(i64::MAX as u128) as i64,
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
        let mut payload = Vec::new();
        request
            .write_to_vec(&mut payload)
            .map_err(|_| GuestControlHealthError::Protocol)?;
        let response = self
            .client
            .request(ttrpc::Request {
                service: "nixling.guest.v1.GuestControl".to_owned(),
                method: method.to_owned(),
                timeout_nano: self.timeout_nano,
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
    async fn hello(&self, request: pb::HelloRequest) -> Result<pb::HelloResponse, GuestControlHealthError> {
        self.unary("Hello", request).await
    }

    async fn authenticate(
        &self,
        request: pb::AuthenticateRequest,
    ) -> Result<pb::AuthenticateResponse, GuestControlHealthError> {
        self.unary("Authenticate", request).await
    }

    async fn health(&self, request: pb::HealthRequest) -> Result<pb::HealthResponse, GuestControlHealthError> {
        self.unary("Health", request).await
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

    struct FakeClient {
        bad_guest_tag: bool,
        overlong_boot_id: bool,
        invalid_health: bool,
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
            response.guest_auth_tag = Some(vec![if self.bad_guest_tag { 0x99 } else { 0x77 }; AUTH_TAG_LEN]);
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
            health.remediation = protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
            health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
            Ok(health)
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
                },
                &FakeSigner,
            )
            .await,
            Err(GuestControlHealthError::Protocol)
        ));
    }
}
