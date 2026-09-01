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
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, SchemaFingerprint, ZoneId,
    identity::{
        AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
        ServiceName, SessionBinding, SessionPurpose,
    },
};
use d2b_contracts_zone_session::v3::component_session::{
    AuthorizationLease, EndpointPolicy, EndpointPolicyIdentity,
};
use d2b_contracts_zone_session::v3::zone_routing::ZoneSigningKeyFingerprint;
use d2b_session::{
    AuthenticatedSessionRouteBinding, HandshakeCredentials, Secret32, SessionAcceptor,
    SessionAuthenticationBinding, SessionAuthorizationRequest, SessionEngine, SessionTtrpcClient,
    TransportEvidence,
};
use d2b_session_unix::FramedVsockTransport;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::{
    component_session_vsock::{
        ComponentSessionTransportProbeResult, connect_component_session_vsock,
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
    pub fn read_from_state_root(
        state_root: impl AsRef<Path>,
    ) -> Result<Self, GuestComponentSessionError> {
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
        let endpoint_root = fs::canonicalize(&endpoint.state_root)
            .map_err(|_| GuestComponentSessionError::DescriptorUnavailable)?;
        let supplied_root = fs::canonicalize(state_root)
            .map_err(|_| GuestComponentSessionError::DescriptorUnavailable)?;
        if endpoint_root != supplied_root {
            return Err(GuestComponentSessionError::DescriptorInvalid);
        }
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
    peer_key_fingerprint: ZoneSigningKeyFingerprint,
    route_binding: AuthenticatedSessionRouteBinding,
    driver: Arc<dyn d2b_session::ComponentSessionDriver>,
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
            connect_component_session_vsock(
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
            ComponentSessionTransportProbeResult::Connected(stream) => stream,
            ComponentSessionTransportProbeResult::Failed(failure) => {
                tracing::warn!(
                    failure = ?failure,
                    "Guest ComponentSession transport connection failed",
                );
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
        .inspect_err(|error| {
            tracing::warn!(
                error = ?error,
                "Guest ComponentSession Noise handshake failed",
            );
        })
        .map_err(|_| GuestComponentSessionClientError::Session)?;
        let generation = engine.generation();
        if generation < identity.reconnect_generation().get() {
            return Err(GuestComponentSessionClientError::StaleSession);
        }
        fn connected_stream_to_transport(
            connected: crate::component_session_vsock::ComponentSessionConnectedStream,
        ) -> Result<FramedVsockTransport<tokio::net::UnixStream>, GuestComponentSessionClientError>
        {
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
        let policy = identity.endpoint_policy_for_generation(generation);
        let acceptor_policy: EndpointPolicy = policy.clone();
        let expected_identity = identity.clone();
        let expected_guest_public = guest_public;
        let authorize_identity = identity.clone();
        let authenticated = SessionAcceptor::from_verified_adapter(
            acceptor_policy,
            identity.zone().clone(),
            move |_evidence, binding, expected_zone, now_tick| {
                authenticate_guest_peer(
                    &expected_identity,
                    expected_guest_public,
                    binding,
                    now_tick,
                    expected_zone,
                )
            },
            move |subject, request, previous, now_tick| {
                authorize_guest_peer(subject, request, previous, now_tick, &authorize_identity)
            },
            (),
        )
        .map_err(|_| GuestComponentSessionClientError::Session)?;
        let evidence = TransportEvidence::new(
            EvidenceClass::EnrolledKk,
            BindingDigest::parse(format!("sha256:{}", hex_digest(identity.channel_binding())))
                .map_err(|_| GuestComponentSessionClientError::Session)?,
        );
        let session = authenticated
            .admit(engine, evidence, monotonic_tick())
            .await
            .map_err(|_| GuestComponentSessionClientError::Session)?;
        let route_binding = session.route_binding();
        let driver: Arc<dyn d2b_session::ComponentSessionDriver> =
            Arc::new(session.into_authenticated_driver());
        let transport = SessionTtrpcClient::new(Arc::clone(&driver));
        let peer_key_fingerprint =
            ZoneSigningKeyFingerprint::parse(format!("sha256.{}", hex_digest(guest_public)))
                .map_err(|_| GuestComponentSessionClientError::Session)?;
        Ok(Self {
            identity,
            session_generation: generation,
            peer_key_fingerprint,
            route_binding,
            driver,
            transport,
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

    /// Return the enrolled peer key fingerprint used by the ZoneLink
    /// controller's child-local session state.
    pub fn peer_key_fingerprint(&self) -> &ZoneSigningKeyFingerprint {
        &self.peer_key_fingerprint
    }

    /// Return the route metadata produced by the authenticated handshake.
    pub fn route_binding(&self) -> AuthenticatedSessionRouteBinding {
        self.route_binding.clone()
    }

    /// Borrow the authenticated driver retained by this client.
    pub fn driver(&self) -> Arc<dyn d2b_session::ComponentSessionDriver> {
        Arc::clone(&self.driver)
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

fn authenticate_guest_peer(
    identity: &GuestIdentity,
    expected_guest_public: [u8; 32],
    binding: &SessionAuthenticationBinding,
    now_tick: u64,
    expected_zone: &ZoneId,
) -> d2b_session::Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
    let policy = identity.endpoint_policy_for_generation(binding.reconnect_generation().get());
    let expected_service = ServiceName::parse(policy.service.as_str()).map_err(|_| {
        d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
    })?;
    if expected_zone != identity.zone()
        || binding.evidence_class() != EvidenceClass::EnrolledKk
        || binding.remote_static_key() != Some(&expected_guest_public)
        || binding.purpose().as_str() != identity.purpose().as_str()
        || binding.purpose_class() != policy.purpose_class
        || binding.initiator_role() != policy.initiator_role
        || binding.responder_role() != policy.responder_role
        || binding.endpoint_locality() != policy.transport_binding.locality
        || binding.service() != &expected_service
        || binding.schema_fingerprint() != identity.schema_fingerprint()
        || binding.reconnect_generation().get() < identity.reconnect_generation().get()
        || binding.transport_class() != policy.transport_binding.transport
        || binding.transport_binding().locality() != Locality::Local
        || binding.transport_binding().binding_digest()
            != &BindingDigest::parse(format!("sha256:{}", hex_digest(identity.channel_binding())))
                .map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            })?
    {
        return Err(d2b_session::SessionError::new(
            d2b_session::contract::SessionErrorCode::PolicyDenied,
        ));
    }
    let session = SessionBinding::new(
        identity.schema_fingerprint().clone(),
        binding.transport_binding().clone(),
        binding.reconnect_generation(),
        binding.transcript_hash().clone(),
    );
    let zone_ref = ResourceRef::new(
        ResourceTypeName::parse("Zone").expect("Zone type"),
        ResourceName::parse(identity.zone().as_str()).expect("Zone name"),
    );
    let context = AuthenticatedSubjectContext::new(
        identity.guest_ref().clone(),
        identity.guest_uid().clone(),
        zone_ref,
        EvidenceClass::EnrolledKk,
        identity.purpose().clone(),
        expected_service,
        session,
    )
    .with_execution_ref(identity.guest_ref().clone())
    .with_provider_generation(
        d2b_contracts_resource::v3::ResourceGeneration::new(identity.provider_generation())
            .map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            })?,
    )
    .with_controller_generation(
        d2b_contracts_resource::v3::ControllerGeneration::new(identity.controller_generation())
            .map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            })?,
    );
    let expiry = now_tick.checked_add(60_000).ok_or_else(|| {
        d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
    })?;
    AuthorizationLease::new(1, expiry)
        .map_err(|_| {
            d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
        })
        .map(|lease| (context, lease))
}

fn authorize_guest_peer(
    subject: &AuthenticatedSubjectContext,
    request: &SessionAuthorizationRequest,
    previous: AuthorizationLease,
    now_tick: u64,
    identity: &GuestIdentity,
) -> d2b_session::Result<AuthorizationLease> {
    use d2b_resource_api::authz::SessionVerb;

    let expected_service = ServiceName::parse("d2b.resource.v3").map_err(|_| {
        d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
    })?;
    let target_is_allowed = request.target().is_none_or(|target| {
        target.resource_type().as_str() != "Guest" || target == identity.guest_ref()
    });
    if subject.subject_ref() != identity.guest_ref()
        || subject.subject_uid() != identity.guest_uid()
        || subject.zone_ref().resource_type().as_str() != "Zone"
        || subject.zone_ref().name().as_str() != identity.zone().as_str()
        || subject.execution_ref() != Some(identity.guest_ref())
        || subject.session_purpose() != identity.purpose()
        || subject.service() != &expected_service
        || subject.evidence_class() != EvidenceClass::EnrolledKk
        || request.target_zone() != identity.zone()
        || request.service() != &expected_service
        || !target_is_allowed
        || !matches!(
            request.verb(),
            SessionVerb::Invoke
                | SessionVerb::OpenStream
                | SessionVerb::Observe
                | SessionVerb::Cancel
        )
        || !previous.is_valid_at(now_tick)
    {
        return Err(d2b_session::SessionError::new(
            d2b_session::contract::SessionErrorCode::PolicyDenied,
        ));
    }
    Ok(previous)
}

fn hex_digest(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn monotonic_tick() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(1)
        .max(1)
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

    #[test]
    fn config_rejects_a_state_root_that_differs_from_the_endpoint_root() {
        let root = std::env::current_dir()
            .expect("test working directory")
            .join("target")
            .join(format!("component-session-roots-{}", std::process::id()));
        let state_root = root.join("state");
        let endpoint_root = root.join("endpoint");
        let component_dir = state_root.join("component-session");
        fs::create_dir_all(&component_dir).expect("component-session directory");
        fs::create_dir_all(&endpoint_root).expect("endpoint directory");
        let descriptor = GuestComponentSessionDescriptor {
            guest_ref: "Guest/workload".to_owned(),
            guest_uid: "123e4567-e89b-42d3-a456-426614174000".to_owned(),
            zone: "work".to_owned(),
            boot_identity_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
            purpose: crate::guest_mode::GUEST_COMPONENT_SESSION_PURPOSE.to_owned(),
            schema_fingerprint:
                "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
            reconnect_generation: 1,
            provider_generation: 1,
            controller_generation: 1,
            assignment_epoch: 1,
        };
        fs::write(
            component_dir.join(
                GUEST_COMPONENT_SESSION_DESCRIPTOR
                    .rsplit('/')
                    .next()
                    .unwrap(),
            ),
            serde_json::to_vec(&descriptor).expect("descriptor JSON"),
        )
        .expect("write descriptor");
        let endpoint = GuestComponentSessionEndpoint {
            socket_path: endpoint_root.join("vsock.sock"),
            state_root: endpoint_root.clone(),
            expected_state_root_uid: 0,
            expected_state_root_gid: 0,
            expected_peer_uid: 0,
            expected_peer_gid: 0,
            setup_timeout: COMPONENT_SESSION_ATTEMPT_CAP,
        };
        assert_eq!(
            GuestComponentSessionConfig::from_state_root(&state_root, endpoint).err(),
            Some(GuestComponentSessionError::DescriptorInvalid)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
