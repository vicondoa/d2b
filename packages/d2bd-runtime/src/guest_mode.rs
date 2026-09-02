//! Provider-neutral Guest target runtime.
//!
//! Guest mode owns only the enrolled parent-Zone ComponentSession and the
//! target-local ProviderDeployment. It deliberately has no constructor for a
//! local Zone store, public operator socket, realm credential, or Host
//! authority.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Instant,
};

use d2b_contracts_resource::v3::{
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, SchemaFingerprint, ZoneId,
    identity::{
        AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
        ServiceName, SessionBinding, SessionPurpose,
    },
};
use d2b_contracts_zone_session::v3::component_session::{
    AttachmentPolicy, ComponentSessionPreface, EndpointPolicy, EndpointPolicyIdentity,
    EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, LimitProfile, NoiseProfile,
    PurposeClass, ServicePackage, TransportBinding, TransportClass,
};
use d2b_session::{
    AuthenticatedComponentSession, HandshakeCredentials, Secret32, SessionAcceptor,
    SessionAuthenticationBinding, SessionAuthorizationRequest, SessionEngine, TransportEvidence,
};
use d2b_session_unix::NativeVsockListener;
use sha2::{Digest, Sha256};

use crate::{
    broker_transport::{ModeBoundBrokerAdapter, ModeBoundBrokerError},
    guest_resource_runtime::{GuestResourceRuntime, GuestResourceRuntimeError},
    target_runtime::{
        AdmissionBudget, AdmissionError, AdmissionKind, AdmissionLimits, AssignmentLease,
        ControllerAssignmentKey, DaemonMode, DeploymentError, ProviderDeployment,
    },
};

/// The enrolled Guest ComponentSession listener port.
pub const GUEST_COMPONENT_SESSION_PORT: u32 = 14_318;
/// The kernel source of truth for boot identity.
pub const KERNEL_BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
/// The fixed enrolled session purpose.
pub const GUEST_COMPONENT_SESSION_PURPOSE: &str = "zone-link";
/// The parent-Zone service package carried by the session.
pub const GUEST_COMPONENT_SESSION_SERVICE: ServicePackage = ServicePackage::ResourceV3;
/// The session schema domain used by the Guest target agent.
pub const GUEST_COMPONENT_SESSION_SCHEMA_DOMAIN: &[u8] = b"d2b-guest-component-session-v3";

/// Reject a retired feature-specific Guest prelude before any
/// feature payload or per-session state is allocated.
pub fn reject_legacy_guest_prelude(bytes: &[u8]) -> Result<(), GuestModeError> {
    if bytes.starts_with(crate::component_session_vsock::COMPONENT_SESSION_CONNECT_LINE)
        || bytes.starts_with(b"D2BGC")
    {
        return Err(GuestModeError::OldProtocol);
    }
    if bytes.len() < d2b_contracts_zone_session::v3::component_session::PREFACE_LEN {
        return Err(GuestModeError::OldProtocol);
    }
    ComponentSessionPreface::parse(
        &bytes[..d2b_contracts_zone_session::v3::component_session::PREFACE_LEN],
    )
    .map(|_| ())
    .map_err(|_| GuestModeError::OldProtocol)
}

/// A kernel-derived boot identity. The raw boot-id text never leaves the
/// constructor; only its domain-separated digest participates in binding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BootIdentity([u8; 32]);

impl BootIdentity {
    pub fn from_kernel_boot_id(value: &str) -> Result<Self, GuestModeError> {
        let value = value.trim();
        if value.is_empty() || value.len() > 128 || !value.is_ascii() {
            return Err(GuestModeError::BootIdentityInvalid);
        }
        let mut digest = Sha256::new();
        digest.update(b"d2b-kernel-boot-id-v1\0");
        digest.update(value.as_bytes());
        Ok(Self(digest.finalize().into()))
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self, GuestModeError> {
        let value =
            fs::read_to_string(path).map_err(|_| GuestModeError::BootIdentityUnavailable)?;
        Self::from_kernel_boot_id(&value)
    }

    /// Rehydrate a previously published boot-identity commitment.
    ///
    /// Hosts never need the raw kernel boot-id to bind a reconnect. They may
    /// persist only this domain-separated digest and use it to reconstruct
    /// the enrolled identity after a daemon restart.
    pub fn from_digest(value: &str) -> Result<Self, GuestModeError> {
        let encoded = value
            .strip_prefix("sha256:")
            .ok_or(GuestModeError::BootIdentityInvalid)?;
        if encoded.len() != 64 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GuestModeError::BootIdentityInvalid);
        }
        let mut digest = [0_u8; 32];
        for (index, slot) in digest.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
                .map_err(|_| GuestModeError::BootIdentityInvalid)?;
        }
        Ok(Self(digest))
    }

    pub const fn digest(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Debug for BootIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BootIdentity(<redacted>)")
    }
}

/// The complete trusted identity a Guest must bind to its parent session.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestIdentity {
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    zone: ZoneId,
    boot_identity: BootIdentity,
    purpose: SessionPurpose,
    schema_fingerprint: SchemaFingerprint,
    reconnect_generation: ReconnectGeneration,
    provider_generation: u64,
    controller_generation: u64,
    assignment_epoch: u64,
}

impl GuestIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        zone: ZoneId,
        boot_identity: BootIdentity,
        purpose: SessionPurpose,
        schema_fingerprint: SchemaFingerprint,
        reconnect_generation: ReconnectGeneration,
        provider_generation: u64,
        controller_generation: u64,
        assignment_epoch: u64,
    ) -> Result<Self, GuestModeError> {
        if guest_ref.resource_type().as_str() != "Guest" {
            return Err(GuestModeError::GuestIdentityWrongKind);
        }
        if provider_generation == 0 || controller_generation == 0 || assignment_epoch == 0 {
            return Err(GuestModeError::GenerationZero);
        }
        if purpose.as_str() != GUEST_COMPONENT_SESSION_PURPOSE {
            return Err(GuestModeError::PurposeMismatch);
        }
        Ok(Self {
            guest_ref,
            guest_uid,
            zone,
            boot_identity,
            purpose,
            schema_fingerprint,
            reconnect_generation,
            provider_generation,
            controller_generation,
            assignment_epoch,
        })
    }

    pub fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    pub fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    pub fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub fn boot_identity(&self) -> BootIdentity {
        self.boot_identity
    }

    pub fn purpose(&self) -> &SessionPurpose {
        &self.purpose
    }

    pub fn schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.schema_fingerprint
    }

    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    pub const fn provider_generation(&self) -> u64 {
        self.provider_generation
    }

    pub const fn controller_generation(&self) -> u64 {
        self.controller_generation
    }

    pub const fn assignment_epoch(&self) -> u64 {
        self.assignment_epoch
    }

    /// Domain-separated binding for the enrolled Guest identity and
    /// ComponentSession transcript. Generation is a separate exact session
    /// field so reconnects can advance without changing the enrolled binding.
    pub fn channel_binding(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-guest-component-session-binding-v3\0");
        digest.update(self.guest_ref.to_canonical_string().as_bytes());
        digest.update(self.guest_uid.as_str().as_bytes());
        digest.update(self.zone.as_str().as_bytes());
        digest.update(self.purpose.as_str().as_bytes());
        digest.update(self.schema_fingerprint.as_str().as_bytes());
        digest.update(self.boot_identity.0);
        digest.update(self.provider_generation.to_be_bytes());
        digest.update(self.controller_generation.to_be_bytes());
        digest.update(self.assignment_epoch.to_be_bytes());
        digest.finalize().into()
    }

    pub fn channel_binding_for_generation(&self, _generation: u64) -> [u8; 32] {
        self.channel_binding()
    }

    /// Build the exact enrolled ComponentSession policy expected from the
    /// parent Zone. No caller-controlled field is accepted at admission time.
    pub fn endpoint_policy(&self) -> EndpointPolicy {
        self.endpoint_policy_for_generation(self.reconnect_generation.get())
    }

    pub fn endpoint_policy_for_generation(&self, generation: u64) -> EndpointPolicy {
        EndpointPolicy {
            purpose: EndpointPurpose::ZoneLink,
            purpose_class: PurposeClass::Enrolled,
            initiator_role: EndpointRole::ZoneController,
            responder_role: EndpointRole::GuestAgent,
            service: GUEST_COMPONENT_SESSION_SERVICE,
            schema_fingerprint: digest_bytes(self.schema_fingerprint.as_str()),
            noise_profile: NoiseProfile::Kk25519ChaChaPolySha256,
            limits: LimitProfile::remote_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::NativeVsock,
                locality: d2b_contracts_zone_session::v3::component_session::Locality::GuestLocal,
                channel_binding: self.channel_binding(),
                identity_evidence: IdentityEvidenceRequirement::EnrolledStaticKeys,
            },
            reconnect_generation: generation,
            attachment_policy: AttachmentPolicy::disabled(),
        }
    }

    /// Validate the redacted route metadata produced by ComponentSession
    /// admission before any ProviderDeployment state is allocated.
    pub fn validate_route(
        &self,
        binding: &d2b_session::AuthenticatedSessionRouteBinding,
    ) -> Result<(), GuestModeError> {
        let generation = binding.reconnect_generation().get();
        let policy = self.endpoint_policy_for_generation(generation);
        let zone_ref = binding.context().zone_ref();
        let provider_generation = binding
            .context()
            .provider_generation()
            .is_some_and(|value| value.get() == self.provider_generation);
        let controller_generation = binding
            .context()
            .controller_generation()
            .is_some_and(|value| value.get() == self.controller_generation);
        if !binding.liveness().is_live()
            || generation < self.reconnect_generation.get()
            || binding.zone() != &self.zone
            || binding.subject_ref() != &self.guest_ref
            || binding.subject_uid() != &self.guest_uid
            || binding.schema() != &self.schema_fingerprint
            || binding.service().as_str() != policy.service.as_str()
            || binding.context().session_purpose() != &self.purpose
            || binding.locality() != Locality::Local
            || binding.evidence_class() != EvidenceClass::EnrolledKk
            || binding.purpose_class() != policy.purpose_class
            || binding.initiator_role() != policy.initiator_role
            || binding.responder_role() != policy.responder_role
            || binding.endpoint_locality() != policy.transport_binding.locality
            || binding.transport_class() != policy.transport_binding.transport
            || zone_ref.resource_type().as_str() != "Zone"
            || zone_ref.name().as_str() != self.zone.as_str()
            || binding.context().execution_ref() != Some(&self.guest_ref)
            || binding.context().evidence_class() != EvidenceClass::EnrolledKk
            || binding.context().service().as_str() != policy.service.as_str()
            || binding.context().schema_fingerprint() != &self.schema_fingerprint
            || binding.context().reconnect_generation() != binding.reconnect_generation()
            || binding.context().transport_binding() != binding.transport_binding()
            || !provider_generation
            || !controller_generation
        {
            return Err(GuestModeError::SessionBindingMismatch);
        }
        let expected =
            BindingDigest::parse(format!("sha256:{}", hex_digest(self.channel_binding())))
                .map_err(|_| GuestModeError::SessionBindingMismatch)?;
        if binding.transport_binding().binding_digest() != &expected
            || binding.context().transport_binding().binding_digest() != &expected
        {
            return Err(GuestModeError::SessionBindingMismatch);
        }
        Ok(())
    }
}

impl std::fmt::Debug for GuestIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GuestIdentity(<redacted>)")
    }
}

fn digest_bytes(value: &str) -> [u8; 32] {
    let raw = value.strip_prefix("sha256:").unwrap_or(value);
    let mut bytes = [0_u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
            .expect("validated SHA-256 fingerprint");
    }

    bytes
}

fn hex_digest(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A Guest session lease. Dropping it closes the ComponentSession generation
/// and revokes all target-local controller assignments bound to it.
#[derive(Debug)]
pub struct GuestSessionLease {
    runtime: Weak<GuestRuntimeInner>,
    generation: u64,
    _session_permit: crate::target_runtime::AdmissionPermit,
}

impl GuestSessionLease {
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for GuestSessionLease {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            let _ = runtime.close_generation(self.generation);
        }
    }
}

/// Admission reserved before a ComponentSession handshake allocates engine,
/// Noise, or per-session request state.
#[derive(Debug)]
pub struct GuestHandshakeAdmission {
    session_permit: crate::target_runtime::AdmissionPermit,
    reconnect_permit: Option<crate::target_runtime::AdmissionPermit>,
}

#[derive(Debug)]
struct GuestRuntimeInner {
    identity: GuestIdentity,
    resource_runtime: GuestResourceRuntime,
    deployment: ProviderDeployment,
    admission: AdmissionBudget,
    broker: ModeBoundBrokerAdapter,
    active_generation: Arc<Mutex<Option<u64>>>,
    last_generation: Mutex<u64>,
    active_session_permit: Mutex<Option<crate::target_runtime::AdmissionPermit>>,
}

/// Provider-neutral Guest runtime. It owns no local Zone store or public
/// operator surface.
#[derive(Debug, Clone)]
pub struct GuestRuntime {
    inner: Arc<GuestRuntimeInner>,
}

impl GuestRuntime {
    pub async fn new(
        identity: GuestIdentity,
        broker_socket: PathBuf,
        broker_uid: u32,
        limits: AdmissionLimits,
        state_dir: impl AsRef<Path>,
    ) -> Result<Self, GuestModeError> {
        let admission = AdmissionBudget::new(limits).map_err(GuestModeError::Admission)?;
        let deployment = ProviderDeployment::new(DaemonMode::Guest, limits)
            .map_err(GuestModeError::Admission)?;
        let resource_runtime = GuestResourceRuntime::new(identity.clone(), state_dir)
            .await
            .map_err(GuestModeError::Resource)?;
        let active_generation = resource_runtime.active_generation();
        let broker = ModeBoundBrokerAdapter::guest(broker_socket, broker_uid);
        broker.validate_instance().map_err(GuestModeError::Broker)?;
        let initial_generation = identity.reconnect_generation().get().saturating_sub(1);
        Ok(Self {
            inner: Arc::new(GuestRuntimeInner {
                identity,
                resource_runtime,
                deployment,
                admission,
                broker,
                active_generation,
                last_generation: Mutex::new(initial_generation),
                active_session_permit: Mutex::new(None),
            }),
        })
    }

    pub fn identity(&self) -> &GuestIdentity {
        &self.inner.identity
    }

    /// Return the target-local Resource API owned by this Guest runtime.
    ///
    /// Keeping this alongside the session admission state prevents callers
    /// from accidentally constructing a second store with a mismatched
    /// identity.
    pub fn resource_runtime(&self) -> &GuestResourceRuntime {
        &self.inner.resource_runtime
    }

    pub fn deployment(&self) -> &ProviderDeployment {
        &self.inner.deployment
    }

    pub fn broker(&self) -> &ModeBoundBrokerAdapter {
        &self.inner.broker
    }

    fn next_generation(&self) -> Result<u64, GuestModeError> {
        self.inner
            .last_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?
            .checked_add(1)
            .ok_or(GuestModeError::GenerationZero)
    }

    pub const fn component_session_port() -> u32 {
        GUEST_COMPONENT_SESSION_PORT
    }

    pub const fn surfaces(&self) -> crate::target_runtime::ModeSurfaces {
        DaemonMode::Guest.surfaces()
    }

    /// Host authority surfaces are deliberately unavailable in Guest mode.
    pub fn local_zone_store(&self) -> Result<(), GuestModeError> {
        Err(GuestModeError::HostSurfaceUnavailable)
    }

    pub fn public_operator_socket(&self) -> Result<(), GuestModeError> {
        Err(GuestModeError::HostSurfaceUnavailable)
    }

    pub fn realm_credentials(&self) -> Result<(), GuestModeError> {
        Err(GuestModeError::HostSurfaceUnavailable)
    }

    /// Admit a route only after identity, boot, Zone, purpose, schema, and
    /// generation binding has passed. Reconnects require a new generation and
    /// are bounded before replacing state.
    pub fn admit_route(
        &self,
        binding: &d2b_session::AuthenticatedSessionRouteBinding,
    ) -> Result<GuestSessionLease, GuestModeError> {
        let admission = self.admit_handshake()?;
        self.admit_route_with_handshake(binding, admission)
    }

    /// Reserve the bounded session/reconnect budget before reading or
    /// allocating a ComponentSession handshake.
    pub fn admit_handshake(&self) -> Result<GuestHandshakeAdmission, GuestModeError> {
        let active = self
            .inner
            .active_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?;
        let reconnect_permit = if active.is_some() {
            Some(
                self.inner
                    .admission
                    .try_admit_reconnect(Instant::now())
                    .map_err(GuestModeError::Admission)?,
            )
        } else {
            None
        };
        drop(active);
        let session_permit = self
            .inner
            .admission
            .try_admit(AdmissionKind::Session)
            .map_err(GuestModeError::Admission)?;
        Ok(GuestHandshakeAdmission {
            session_permit,
            reconnect_permit,
        })
    }

    fn admit_route_with_handshake(
        &self,
        binding: &d2b_session::AuthenticatedSessionRouteBinding,
        admission: GuestHandshakeAdmission,
    ) -> Result<GuestSessionLease, GuestModeError> {
        self.inner.identity.validate_route(binding)?;
        let generation = binding.reconnect_generation().get();
        self.admit_generation_with_handshake(generation, admission)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn admit_generation_for_tests(
        &self,
        generation: u64,
    ) -> Result<GuestSessionLease, GuestModeError> {
        if generation < self.inner.identity.reconnect_generation.get() {
            return Err(GuestModeError::StaleSession);
        }
        let admission = self.admit_handshake()?;
        self.admit_generation_with_handshake(generation, admission)
    }

    fn admit_generation_with_handshake(
        &self,
        generation: u64,
        admission: GuestHandshakeAdmission,
    ) -> Result<GuestSessionLease, GuestModeError> {
        let mut last_generation = self
            .inner
            .last_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?;
        if generation <= *last_generation {
            return Err(GuestModeError::StaleSession);
        }
        let mut active = self
            .inner
            .active_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?;
        if let Some(previous) = *active {
            if generation <= previous {
                return Err(GuestModeError::StaleSession);
            }
            self.inner
                .deployment
                .revoke_session(previous)
                .map_err(GuestModeError::Deployment)?;
        } else if admission.reconnect_permit.is_some() {
            return Err(GuestModeError::StaleSession);
        }
        {
            let mut active_permit = self
                .inner
                .active_session_permit
                .lock()
                .map_err(|_| GuestModeError::StateUnavailable)?;
            if let Some(permit) = active_permit.take() {
                permit.release();
            }
            *active_permit = Some(admission.session_permit.clone());
        }
        *active = Some(generation);
        *last_generation = generation;
        Ok(GuestSessionLease {
            runtime: Arc::downgrade(&self.inner),
            generation,
            _session_permit: admission.session_permit,
        })
    }

    /// Close the current session and revoke its target-local assignments.
    pub fn close_session(&self, generation: u64) -> Result<(), GuestModeError> {
        self.inner.close_generation(generation)
    }

    /// Admit a controller assignment only for this Guest's active session.
    pub fn admit_assignment(
        &self,
        key: ControllerAssignmentKey,
    ) -> Result<AssignmentLease, GuestModeError> {
        let active = self
            .inner
            .active_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?;
        if *active != Some(key.session_generation) {
            return Err(GuestModeError::StaleSession);
        }
        self.inner
            .deployment
            .admit_assignment(key)
            .map_err(GuestModeError::Deployment)
    }

    /// Establish a real authenticated responder ComponentSession over the
    /// fixed native vsock transport. The returned session is not a resource
    /// authority until its route lease is retained by the caller.
    pub async fn establish_component_session<T>(
        &self,
        transport: T,
        local_private: Secret32,
        parent_public: [u8; 32],
    ) -> Result<(AuthenticatedComponentSession<()>, GuestSessionLease), GuestModeError>
    where
        T: d2b_session::OwnedTransport + 'static,
    {
        let admission = self.admit_handshake()?;
        let identity = self.inner.identity.clone();
        let minimum_generation = self.next_generation()?;
        let policy = identity.endpoint_policy_for_generation(minimum_generation);
        let engine = SessionEngine::establish_responder_with_generation_floor(
            transport,
            policy.clone(),
            HandshakeCredentials::Kk {
                local_private,
                remote_public: parent_public,
            },
            minimum_generation,
            Instant::now(),
        )
        .await
        .map_err(GuestModeError::Session)?;
        let generation = engine.generation();
        let acceptor_policy = EndpointPolicyIdentity::from(&identity.endpoint_policy())
            .with_generation(generation)
            .map_err(|_| GuestModeError::SessionBindingMismatch)?;
        let expected = identity.clone();
        let authorize_identity = identity.clone();
        let acceptor = SessionAcceptor::from_verified_adapter(
            acceptor_policy,
            identity.zone.clone(),
            move |_evidence, binding, expected_zone, now_tick| {
                authenticate_guest_subject(&expected, binding, now_tick, expected_zone)
            },
            move |subject, request, previous, now_tick| {
                authorize_guest_request(&authorize_identity, subject, request, previous, now_tick)
            },
            (),
        )
        .map_err(GuestModeError::Session)?;
        let session = acceptor
            .admit(
                engine,
                TransportEvidence::new(
                    EvidenceClass::EnrolledKk,
                    BindingDigest::parse(format!(
                        "sha256:{}",
                        hex_digest(identity.channel_binding())
                    ))
                    .map_err(|_| GuestModeError::SessionBindingMismatch)?,
                ),
                monotonic_tick(),
            )
            .await
            .map_err(GuestModeError::Session)?;
        let route = session.route_binding();
        let lease = self.admit_route_with_handshake(&route, admission)?;
        Ok((session, lease))
    }

    /// Bind the fixed Guest listener. Host mode has no equivalent entrypoint.
    pub fn bind_listener(&self) -> Result<NativeVsockListener, GuestModeError> {
        NativeVsockListener::bind(GUEST_COMPONENT_SESSION_PORT).map_err(GuestModeError::Transport)
    }

    /// Accept only the parent Host CID on the fixed Guest listener.
    pub async fn accept_from_parent(
        &self,
        listener: &mut NativeVsockListener,
        local_private: Secret32,
        parent_public: [u8; 32],
    ) -> Result<(AuthenticatedComponentSession<()>, GuestSessionLease), GuestModeError> {
        let transport = listener
            .accept_host()
            .await
            .map_err(GuestModeError::Transport)?;
        self.establish_component_session(transport, local_private, parent_public)
            .await
    }
}

impl GuestRuntimeInner {
    fn close_generation(&self, generation: u64) -> Result<(), GuestModeError> {
        let mut active = self
            .active_generation
            .lock()
            .map_err(|_| GuestModeError::StateUnavailable)?;
        if *active == Some(generation) {
            self.deployment
                .revoke_session(generation)
                .map_err(GuestModeError::Deployment)?;
            if let Ok(mut permit) = self.active_session_permit.lock() {
                if let Some(permit) = permit.take() {
                    permit.release();
                }
            }
            *active = None;
        }
        Ok(())
    }
}

fn authenticate_guest_subject(
    identity: &GuestIdentity,
    binding: &SessionAuthenticationBinding,
    now_tick: u64,
    expected_zone: &ZoneId,
) -> d2b_session::Result<(
    AuthenticatedSubjectContext,
    d2b_contracts_zone_session::v3::component_session::AuthorizationLease,
)> {
    let policy = identity.endpoint_policy_for_generation(binding.reconnect_generation().get());
    let expected_service = ServiceName::parse(policy.service.as_str()).map_err(|_| {
        d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
    })?;
    if expected_zone != &identity.zone
        || binding.evidence_class() != EvidenceClass::EnrolledKk
        || binding.purpose() != identity.purpose()
        || binding.purpose_class() != policy.purpose_class
        || binding.initiator_role() != policy.initiator_role
        || binding.responder_role() != policy.responder_role
        || binding.endpoint_locality() != policy.transport_binding.locality
        || binding.service() != &expected_service
        || binding.schema_fingerprint() != identity.schema_fingerprint()
        || binding.reconnect_generation().get() < identity.reconnect_generation.get()
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
        identity.schema_fingerprint.clone(),
        binding.transport_binding().clone(),
        binding.reconnect_generation(),
        binding.transcript_hash().clone(),
    );
    let context = AuthenticatedSubjectContext::new(
        identity.guest_ref.clone(),
        identity.guest_uid.clone(),
        ResourceRef::new(
            ResourceTypeName::parse("Zone").expect("Zone type"),
            ResourceName::parse(identity.zone.as_str()).expect("Zone name"),
        ),
        EvidenceClass::EnrolledKk,
        identity.purpose.clone(),
        ServiceName::parse("d2b.resource.v3").expect("resource service"),
        session,
    )
    .with_execution_ref(identity.guest_ref.clone())
    .with_provider_generation(
        d2b_contracts_resource::v3::ResourceGeneration::new(identity.provider_generation).map_err(
            |_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            },
        )?,
    )
    .with_controller_generation(
        d2b_contracts_resource::v3::ControllerGeneration::new(identity.controller_generation)
            .map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            })?,
    );
    let expiry = now_tick.checked_add(60_000).ok_or_else(|| {
        d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
    })?;
    let lease =
        d2b_contracts_zone_session::v3::component_session::AuthorizationLease::new(1, expiry)
            .map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_session::contract::SessionErrorCode::PolicyDenied,
                )
            })?;
    Ok((context, lease))
}

fn authorize_guest_request(
    identity: &GuestIdentity,
    subject: &AuthenticatedSubjectContext,
    request: &SessionAuthorizationRequest,
    previous: d2b_contracts_zone_session::v3::component_session::AuthorizationLease,
    now_tick: u64,
) -> d2b_session::Result<d2b_contracts_zone_session::v3::component_session::AuthorizationLease> {
    use d2b_resource_api::authz::SessionVerb;
    let expected_service =
        ServiceName::parse(GUEST_COMPONENT_SESSION_SERVICE.as_str()).map_err(|_| {
            d2b_session::SessionError::new(d2b_session::contract::SessionErrorCode::PolicyDenied)
        })?;
    let subject_zone = subject.zone_ref();
    let subject_generations_match = subject
        .provider_generation()
        .is_some_and(|value| value.get() == identity.provider_generation)
        && subject
            .controller_generation()
            .is_some_and(|value| value.get() == identity.controller_generation);
    let target_is_allowed = request.target().is_none_or(|target| {
        target.resource_type().as_str() != "Guest" || target == &identity.guest_ref
    });
    if subject.subject_ref() != &identity.guest_ref
        || subject.subject_uid() != &identity.guest_uid
        || subject_zone.resource_type().as_str() != "Zone"
        || subject_zone.name().as_str() != identity.zone.as_str()
        || subject.execution_ref() != Some(&identity.guest_ref)
        || subject.session_purpose() != &identity.purpose
        || subject.service() != &expected_service
        || subject.evidence_class() != EvidenceClass::EnrolledKk
        || !subject_generations_match
        || request.target_zone() != &identity.zone
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

fn monotonic_tick() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(1)
        .max(1)
}

/// Guest-mode failures are closed and identity-free.
#[derive(Debug)]
pub enum GuestModeError {
    Admission(AdmissionError),
    Deployment(DeploymentError),
    Resource(GuestResourceRuntimeError),
    GuestIdentityWrongKind,
    BootIdentityInvalid,
    BootIdentityUnavailable,
    GenerationZero,
    PurposeMismatch,
    SessionBindingMismatch,
    StaleSession,
    StateUnavailable,
    HostSurfaceUnavailable,
    OldProtocol,
    Broker(ModeBoundBrokerError),
    Session(d2b_session::SessionError),
    Transport(d2b_session::TransportError),
}

impl std::fmt::Display for GuestModeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Admission(error) => return error.fmt(formatter),
            Self::Deployment(error) => return error.fmt(formatter),
            Self::Resource(error) => return error.fmt(formatter),
            Self::GuestIdentityWrongKind => "guest-mode-identity-wrong-kind",
            Self::BootIdentityInvalid => "guest-mode-boot-identity-invalid",
            Self::BootIdentityUnavailable => "guest-mode-boot-identity-unavailable",
            Self::GenerationZero => "guest-mode-generation-zero",
            Self::PurposeMismatch => "guest-mode-purpose-mismatch",
            Self::SessionBindingMismatch => "guest-mode-session-binding-mismatch",
            Self::StaleSession => "guest-mode-stale-session",
            Self::StateUnavailable => "guest-mode-state-unavailable",
            Self::HostSurfaceUnavailable => "guest-mode-host-surface-unavailable",
            Self::OldProtocol => "guest-mode-old-protocol",
            Self::Broker(error) => return error.fmt(formatter),
            Self::Session(error) => return error.fmt(formatter),
            Self::Transport(_) => "guest-mode-transport-failed",
        })
    }
}

impl std::error::Error for GuestModeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;

    fn identity(generation: u64) -> GuestIdentity {
        GuestIdentity::new(
            ResourceRef::parse("Guest/workload").expect("guest"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid"),
            ZoneId::parse("work").expect("zone"),
            BootIdentity::from_kernel_boot_id("boot-id-for-test").expect("boot"),
            SessionPurpose::parse(GUEST_COMPONENT_SESSION_PURPOSE).expect("purpose"),
            SchemaFingerprint::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            )
            .expect("schema"),
            ReconnectGeneration::new(generation).expect("generation"),
            1,
            1,
            1,
        )
        .expect("identity")
    }

    #[test]
    fn boot_identity_is_kernel_derived_and_redacted() {
        let first = BootIdentity::from_kernel_boot_id("boot-a").expect("boot");
        let second = BootIdentity::from_kernel_boot_id("boot-b").expect("boot");
        assert_ne!(first, second);
        assert!(!format!("{first:?}").contains("boot-a"));
    }

    #[test]
    fn guest_policy_binds_zone_purpose_schema_boot_and_generation() {
        let identity = identity(7);
        let policy = identity.endpoint_policy();
        assert_eq!(policy.reconnect_generation, 7);
        assert_eq!(policy.purpose, EndpointPurpose::ZoneLink);
        assert_eq!(policy.service, ServicePackage::ResourceV3);
        assert_eq!(
            policy.transport_binding.transport,
            TransportClass::NativeVsock
        );
        assert_ne!(policy.transport_binding.channel_binding, [0; 32]);
    }

    #[tokio::test]
    async fn guest_runtime_exposes_no_host_authority_surfaces() {
        let identity = identity(1);
        let state_dir = tempfile::tempdir().expect("state directory");
        let runtime = GuestRuntime::new(
            identity,
            PathBuf::from("/run/d2b/guest-broker.sock"),
            997,
            AdmissionLimits::guest_default(),
            state_dir.path(),
        )
        .await
        .expect("runtime");
        assert!(!runtime.surfaces().local_zone_store);
        assert!(runtime.local_zone_store().is_err());
        assert!(runtime.public_operator_socket().is_err());
        assert!(runtime.realm_credentials().is_err());
    }

    #[test]
    fn retired_guest_prelude_is_rejected_before_allocation() {
        assert!(matches!(
            reject_legacy_guest_prelude(b"CONNECT 14318\n"),
            Err(GuestModeError::OldProtocol)
        ));
        assert!(matches!(
            reject_legacy_guest_prelude(b"D2BGC-old"),
            Err(GuestModeError::OldProtocol)
        ));
    }
}
