//! Host-side initiator for the enrolled Guest ComponentSession.

use std::{
    fs,
    io::Read,
    os::{
        fd::OwnedFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
    identity::{ReconnectGeneration, SessionPurpose},
};
use d2b_contracts_zone_session::v3::component_session::EndpointPolicyIdentity;
use d2b_session::{HandshakeCredentials, Secret32, SessionEngine, SessionTtrpcClient};
use d2b_session_unix::FramedVsockTransport;
use serde::{Deserialize, Serialize};

use crate::{
    guest_control_vsock::{
        GuestControlTransportProbeResult, connect_guest_control_vsock,
    },
    guest_mode::{BootIdentity, GuestIdentity},
};

/// Relative location of the host-published Guest session descriptor.
///
/// The descriptor contains only non-secret identity and locator metadata.
/// Key material stays in separate files under the same state root.
pub const GUEST_COMPONENT_SESSION_DESCRIPTOR: &str = "component-session/guest.json";
/// Relative location of the host-side ComponentSession private key.
pub const GUEST_COMPONENT_SESSION_PRIVATE_KEY: &str = "component-session/host.key";
/// Relative location of the enrolled Guest public key.
pub const GUEST_COMPONENT_SESSION_GUEST_PUBLIC_KEY: &str = "component-session/guest.pub";
/// Bounded ComponentSession connection attempt.
pub const COMPONENT_SESSION_ATTEMPT_CAP: Duration = Duration::from_secs(3);
/// Backoff between bounded ComponentSession readiness attempts.
pub const COMPONENT_SESSION_RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// Host-published, non-secret identity needed to reconnect to one Guest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestComponentSessionDescriptor {
    pub guest_ref: String,
    pub guest_uid: String,
    pub zone: String,
    pub boot_identity_digest: String,
    pub purpose: String,
    pub schema_fingerprint: String,
    pub reconnect_generation: u64,
    pub provider_generation: u64,
    pub controller_generation: u64,
    pub assignment_epoch: u64,
}

impl GuestComponentSessionDescriptor {
    /// Read and validate a host-published descriptor from one state root.
    pub fn read_from_state_root(state_root: impl AsRef<Path>) -> Result<Self, GuestComponentSessionError> {
        let state_root = state_root.as_ref();
        let path = state_root.join(GUEST_COMPONENT_SESSION_DESCRIPTOR);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| GuestComponentSessionError::DescriptorUnavailable)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.nlink() != 1 {
            return Err(GuestComponentSessionError::DescriptorUnavailable);
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| GuestComponentSessionError::DescriptorUnavailable)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| GuestComponentSessionError::DescriptorUnavailable)?;
        serde_json::from_slice(&bytes).map_err(|_| GuestComponentSessionError::DescriptorInvalid)
    }

    /// Build the enrolled identity represented by this descriptor.
    pub fn identity(&self) -> Result<GuestIdentity, GuestComponentSessionError> {
        let guest_ref = ResourceRef::parse(&self.guest_ref)
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let guest_uid = ResourceUid::parse(self.guest_uid.clone())
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let zone =
            ZoneId::parse(&self.zone).map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let boot_identity = BootIdentity::from_digest(&self.boot_identity_digest)
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let purpose = SessionPurpose::parse(&self.purpose)
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let schema_fingerprint = SchemaFingerprint::parse(&self.schema_fingerprint)
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        let reconnect_generation = ReconnectGeneration::new(self.reconnect_generation)
            .map_err(|_| GuestComponentSessionError::DescriptorInvalid)?;
        GuestIdentity::new(
            guest_ref,
            guest_uid,
            zone,
            boot_identity,
            purpose,
            schema_fingerprint,
            reconnect_generation,
            self.provider_generation,
            self.controller_generation,
            self.assignment_epoch,
        )
        .map_err(|_| GuestComponentSessionError::DescriptorInvalid)
    }
}

/// Validated Cloud Hypervisor CONNECT endpoint for a Guest session.
#[derive(Debug, Clone)]
pub struct GuestComponentSessionEndpoint {
    /// Cloud Hypervisor API Unix socket.
    pub socket_path: PathBuf,
    /// State root containing the socket and its ownership marker.
    pub state_root: PathBuf,
    /// Expected state-root owner UID.
    pub expected_state_root_uid: u32,
    /// Expected state-root owner GID.
    pub expected_state_root_gid: u32,
    /// Expected Cloud Hypervisor peer UID.
    pub expected_peer_uid: u32,
    /// Expected Cloud Hypervisor peer GID.
    pub expected_peer_gid: u32,
    /// Deadline for CONNECT and ACK.
    pub setup_timeout: Duration,
}

/// Fully resolved host-side ComponentSession connection material.
///
/// The descriptor is non-secret; key bytes are loaded only from the
/// host-owned state root and are never serialized or included in diagnostics.
#[derive(Debug)]
pub struct GuestComponentSessionConfig {
    pub identity: GuestIdentity,
    pub endpoint: GuestComponentSessionEndpoint,
    pub local_private: Secret32,
    pub guest_public: [u8; 32],
}

impl GuestComponentSessionConfig {
    /// Resolve the descriptor and enrolled keys from one validated state root.
    pub fn from_state_root(
        state_root: impl AsRef<Path>,
        endpoint: GuestComponentSessionEndpoint,
    ) -> Result<Self, GuestComponentSessionError> {
        let state_root = state_root.as_ref();
        let descriptor = GuestComponentSessionDescriptor::read_from_state_root(state_root)?;
        let identity = descriptor.identity()?;
        let private_path = state_root.join(GUEST_COMPONENT_SESSION_PRIVATE_KEY);
        let public_path = state_root.join(GUEST_COMPONENT_SESSION_GUEST_PUBLIC_KEY);
        let local_private = read_private_key(&private_path)?;
        let guest_public = read_public_key(&public_path)?;
        Ok(Self {
            identity,
            endpoint,
            local_private,
            guest_public,
        })
    }

    /// Establish the authenticated session represented by this configuration.
    pub async fn connect(
        self,
    ) -> Result<GuestComponentSessionClient, GuestComponentSessionClientError> {
        GuestComponentSessionClient::connect(
            self.identity,
            self.endpoint,
            self.local_private,
            self.guest_public,
        )
        .await
    }
}

/// A generated-ttrpc client backed by one authenticated Guest session.
pub struct GuestComponentSessionClient {
    identity: GuestIdentity,
    session_generation: u64,
    transport: SessionTtrpcClient,
}

impl std::fmt::Debug for GuestComponentSessionClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GuestComponentSessionClient(<redacted>)")
    }
}

impl GuestComponentSessionClient {
    /// Connect using an already validated identity, endpoint, and keys.
    /// Connect through the validated Cloud Hypervisor CONNECT endpoint.
    pub async fn connect(
        identity: GuestIdentity,
        endpoint: GuestComponentSessionEndpoint,
        local_private: Secret32,
        guest_public: [u8; 32],
    ) -> Result<Self, GuestComponentSessionClientError> {
        let connected = tokio::task::spawn_blocking(move || {
            connect_guest_control_vsock(
                &endpoint.socket_path,
                &endpoint.state_root,
                endpoint.expected_state_root_uid,
                endpoint.expected_state_root_gid,
                endpoint.expected_peer_uid,
                endpoint.expected_peer_gid,
                endpoint.setup_timeout,
            )
        })
        .await
        .map_err(|_| GuestComponentSessionClientError::Transport)?;
        let connected = match connected {
            GuestControlTransportProbeResult::Connected(stream) => stream,
            GuestControlTransportProbeResult::Failed(_) => {
                return Err(GuestComponentSessionClientError::Transport);
            }
        };
        let transport = connected_stream_to_transport(connected)?;
        let policy = EndpointPolicyIdentity::from(&identity.endpoint_policy());
        let engine = SessionEngine::establish_initiator_with_generation_discovery(
            transport,
            policy,
            HandshakeCredentials::Kk {
                local_private,
                remote_public: guest_public,
            },
            std::time::Instant::now(),
        )
        .await
        .map_err(|_| GuestComponentSessionClientError::Session)?;
        let generation = engine.generation();
        if generation < identity.reconnect_generation().get() {
            return Err(GuestComponentSessionClientError::StaleSession);
        }

        fn connected_stream_to_transport(
            connected: crate::guest_control_vsock::GuestControlConnectedStream,
        ) -> Result<
            FramedVsockTransport<tokio::net::UnixStream>,
            GuestComponentSessionClientError,
        > {
            let socket = connected.into_socket();
            socket
                .set_read_timeout(None)
                .map_err(|_| GuestComponentSessionClientError::Transport)?;
            socket
                .set_write_timeout(None)
                .map_err(|_| GuestComponentSessionClientError::Transport)?;
            let fd: OwnedFd = socket.into();
            let stream = std::os::unix::net::UnixStream::from(fd);
            stream
                .set_nonblocking(true)
                .map_err(|_| GuestComponentSessionClientError::Transport)?;
            let stream = tokio::net::UnixStream::from_std(stream)
                .map_err(|_| GuestComponentSessionClientError::Transport)?;
            Ok(FramedVsockTransport::new(stream))
        }
        let driver = engine.into_driver();
        Ok(Self {
            identity,
            session_generation: generation,
            transport: SessionTtrpcClient::new(Arc::new(driver)),
        })
    }

    /// The enrolled Guest identity bound into the session transcript.
    pub fn identity(&self) -> &GuestIdentity {
        &self.identity
    }

    /// The reconnect generation authenticated by the session.
    pub fn generation(&self) -> u64 {
        self.session_generation
    }

    /// Clone the generated-ttrpc client for a typed service adapter.
    pub fn client(&self) -> ttrpc::r#async::Client {
        self.transport.client()
    }

    /// Return the generated ResourceService client carried by this
    /// authenticated ComponentSession.
    pub fn resource_service_client(
        &self,
    ) -> d2b_resource_api::generated::d2b_resource_v3_ttrpc::ResourceServiceClient {
        d2b_resource_api::generated::d2b_resource_v3_ttrpc::ResourceServiceClient::new(
            self.transport.client(),
        )
    }
}

/// Errors while loading host-published session metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestComponentSessionError {
    DescriptorUnavailable,
    DescriptorInvalid,
    KeyUnavailable,
    KeyInvalid,
}

impl std::fmt::Display for GuestComponentSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::DescriptorUnavailable => "guest-component-session-descriptor-unavailable",
            Self::DescriptorInvalid => "guest-component-session-descriptor-invalid",
            Self::KeyUnavailable => "guest-component-session-key-unavailable",
            Self::KeyInvalid => "guest-component-session-key-invalid",
        })
    }
}

impl std::error::Error for GuestComponentSessionError {}

fn read_private_key(path: &Path) -> Result<Secret32, GuestComponentSessionError> {
    let bytes = read_key_bytes(path)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| GuestComponentSessionError::KeyInvalid)?;
    Secret32::new(bytes).map_err(|_| GuestComponentSessionError::KeyInvalid)
}

fn read_public_key(path: &Path) -> Result<[u8; 32], GuestComponentSessionError> {
    let bytes = read_key_bytes(path)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| GuestComponentSessionError::KeyInvalid)?;
    if bytes == [0_u8; 32] {
        return Err(GuestComponentSessionError::KeyInvalid);
    }
    Ok(bytes)
}

fn read_key_bytes(path: &Path) -> Result<Vec<u8>, GuestComponentSessionError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GuestComponentSessionError::KeyUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.nlink() != 1 {
        return Err(GuestComponentSessionError::KeyInvalid);
    }
    let bytes = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| GuestComponentSessionError::KeyUnavailable)
        .and_then(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|_| GuestComponentSessionError::KeyUnavailable)?;
            if bytes.len() != 32 {
                return Err(GuestComponentSessionError::KeyInvalid);
            }
            Ok(bytes)
        })?;
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestComponentSessionClientError {
    Transport,
    Session,
    StaleSession,
}

impl std::fmt::Display for GuestComponentSessionClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Transport => "guest-component-session-transport-unavailable",
            Self::Session => "guest-component-session-authentication-failed",
            Self::StaleSession => "guest-component-session-generation-stale",
        })
    }
}

impl std::error::Error for GuestComponentSessionClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_rejects_hardlinked_state_files() {
        let root = std::env::current_dir()
            .expect("test working directory")
            .join(".scratch")
            .join(format!("component-session-{}", std::process::id()));
        let directory = root.join("component-session");
        fs::create_dir_all(&directory).expect("component-session directory");
        let source = root.join("descriptor-source");
        let descriptor = directory.join("guest.json");
        fs::write(&source, b"{}").expect("descriptor source");
        fs::hard_link(&source, &descriptor).expect("descriptor hardlink");
        assert_eq!(
            GuestComponentSessionDescriptor::read_from_state_root(&root),
            Err(GuestComponentSessionError::DescriptorUnavailable)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
