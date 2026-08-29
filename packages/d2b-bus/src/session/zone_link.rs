//! The ZoneLink session: the drive-loop handle the routing and relay slices
//! call through.
//!
//! # What this provides to the slices that were waiting on it
//!
//! Three things the cross-Zone routing and per-hop relay modules could not do
//! without a session layer:
//!
//! - **Opening the next-hop ComponentSession.**
//!   [`ZoneLinkSession::establish_authenticated`] admits a driver only against
//!   a link that reached `Ready` and an exact authenticated ZoneLink profile,
//!   so a next-hop session cannot exist before the enrolled KK handshake
//!   completed.
//! - **Per-hop named-stream credit forwarding.** A relayed stream's credit is
//!   granted on the *next* hop as the local side consumes, which is what keeps
//!   a slow terminal consumer from being paid for by an intermediate Zone's
//!   memory. [`ZoneLinkSession::forward_named_stream_credit`] is that call.
//! - **Actual cancel delivery.** `zone_route::forward_cancel` produces a
//!   delivery *intent*; [`ZoneLinkSession::deliver_cancel`] is the delivery,
//!   bound to the session's own reconnect generation so a cancel can never be
//!   applied across a reconnect fence.
//!
//! # Fencing
//!
//! Every call is gated twice. It refuses unless the link was `Ready` when the
//! session was established, and it refuses after the session is fenced by a
//! disconnect or a revocation. Revocation fencing is what closes long-lived
//! relayed streams with `zone-link-revoked`, and it is one-way: there is no
//! call that unfences a session, so recovery is necessarily a fresh enrolled
//! KK handshake and a fresh session.
//!
//! # No authority
//!
//! The session consumes one verified route admission and its authenticated
//! driver owner. It exposes no admission evidence, subject, capability, or
//! key, resolves nothing, and mints nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use d2b_contracts_zone_session::v3::component_session::{
    CloseReason, Remediation, RequestId, SessionErrorCode,
};
use d2b_session::{
    AuthenticatedComponentSession, AuthenticatedSessionDriver, Cancellation,
    ComponentSessionDriver, StreamId,
};

use crate::session::contract::{RouteAdmissionError, VerifiedRouteAdmission};
use crate::session::enrollment::{LinkEpoch, ZoneLinkEnrollment, ZoneLinkEnrollmentError};

/// A closed refusal raised by a ZoneLink session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkSessionError {
    /// The link had not reached `Ready`; resource traffic is prohibited.
    ResourceTrafficBeforeReady,
    /// The uplink is disconnected. In-flight operations pinned through this
    /// link fail immediately rather than waiting.
    ZoneLinkDisconnected,
    /// The enrollment governing this session was durably revoked.
    ZoneLinkRevoked,
    /// The runtime-owned route admission was revoked or became stale.
    RouteAdmission(RouteAdmissionError),
    /// The underlying session refused, with its closed code.
    Session(SessionErrorCode),
}

impl ZoneLinkSessionError {
    /// The closed, path-free label for this refusal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResourceTrafficBeforeReady => "resource-traffic-before-ready",
            Self::ZoneLinkDisconnected => "zone-link-disconnected",
            Self::ZoneLinkRevoked => "zone-link-revoked",
            Self::RouteAdmission(error) => error.as_str(),
            Self::Session(code) => code.as_str(),
        }
    }
}

impl core::fmt::Display for ZoneLinkSessionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for ZoneLinkSessionError {}

impl From<ZoneLinkEnrollmentError> for ZoneLinkSessionError {
    fn from(value: ZoneLinkEnrollmentError) -> Self {
        match value {
            ZoneLinkEnrollmentError::ZoneLinkRevoked => Self::ZoneLinkRevoked,
            _ => Self::ResourceTrafficBeforeReady,
        }
    }
}

/// The fence state of one ZoneLink session.
///
/// Stored as an atomic so a fence applied by the controller is visible to a
/// concurrently running relayed stream without taking the session by value.
const FENCE_OPEN: u8 = 0;
const FENCE_DISCONNECTED: u8 = 1;
const FENCE_REVOKED: u8 = 2;

/// One established ZoneLink ComponentSession.
///
/// The private driver owner is shared behind an `Arc` only after this type has
/// consumed the authenticated session that created it. Sharing the transport
/// implementation shares no authority: the consumed session owner retains its
/// liveness and single-owner authorization state.
pub struct ZoneLinkSession {
    driver: Arc<dyn ComponentSessionDriver>,
    epoch: LinkEpoch,
    admission: Option<VerifiedRouteAdmission>,
    liveness: Option<d2b_session::SessionLiveness>,
    fence: AtomicU8,
}

impl core::fmt::Debug for ZoneLinkSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The epoch is a link-local ordinal and is safe to show; the driver,
        // its generation, and any stream identity are not.
        formatter
            .debug_struct("ZoneLinkSession")
            .field("epoch", &self.epoch.get())
            .field(
                "fenced",
                &(self.fence.load(Ordering::Acquire) != FENCE_OPEN),
            )
            .finish_non_exhaustive()
    }
}

impl ZoneLinkSession {
    /// Establish a driver lane from one verified route admission and its
    /// owning authenticated ComponentSession.
    ///
    /// The admission is consumed, and the session is consumed into a
    /// non-cloneable driver owner. No caller-supplied policy, subject, Zone,
    /// enrollment identity, or driver handle can substitute for either sealed
    /// input; the enrollment argument is used only for its Ready/epoch gate.
    pub fn establish_authenticated(
        enrollment: &ZoneLinkEnrollment,
        admission: VerifiedRouteAdmission,
        session: AuthenticatedComponentSession<()>,
    ) -> Result<Self, ZoneLinkSessionError> {
        admission.revalidate().map_err(route_admission_error)?;
        enrollment.admit_resource_traffic()?;
        let epoch = enrollment
            .epoch()
            .ok_or(ZoneLinkSessionError::ResourceTrafficBeforeReady)?;
        let route = session.route_binding();
        admission
            .session_binding()
            .matches_authenticated_session(&route)
            .map_err(|_| RouteAdmissionError::SessionBindingMismatch)
            .map_err(route_admission_error)?;
        if !route.liveness().is_live() {
            return Err(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::SessionNotLive,
            ));
        }
        let generation = admission.reconnect_generation().get();
        let driver: AuthenticatedSessionDriver = session.into_authenticated_driver();
        if driver.generation() != generation {
            return Err(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::ReconnectGenerationMismatch,
            ));
        }
        Ok(Self {
            driver: Arc::new(driver),
            epoch,
            admission: Some(admission),
            liveness: Some(route.liveness()),
            fence: AtomicU8::new(FENCE_OPEN),
        })
    }

    #[cfg(test)]
    fn establish_for_tests(
        enrollment: &ZoneLinkEnrollment,
        driver: Arc<dyn ComponentSessionDriver>,
    ) -> Result<Self, ZoneLinkSessionError> {
        enrollment.admit_resource_traffic()?;
        let epoch = enrollment
            .epoch()
            .ok_or(ZoneLinkSessionError::ResourceTrafficBeforeReady)?;
        Ok(Self {
            driver,
            epoch,
            admission: None,
            liveness: None,
            fence: AtomicU8::new(FENCE_OPEN),
        })
    }

    /// The link epoch this session was established under.
    pub const fn epoch(&self) -> LinkEpoch {
        self.epoch
    }

    /// The session's reconnect generation, as the drive loop reports it.
    pub fn generation(&self) -> u64 {
        self.driver.generation()
    }

    /// Whether the session may still carry traffic.
    pub fn is_open(&self) -> bool {
        if self.fence.load(Ordering::Acquire) != FENCE_OPEN
            || self
                .liveness
                .as_ref()
                .is_some_and(|liveness| !liveness.is_live())
        {
            return false;
        }
        if let Some(admission) = &self.admission
            && (admission.revalidate().is_err()
                || self.driver.generation() != admission.reconnect_generation().get())
        {
            self.fence.store(FENCE_REVOKED, Ordering::Release);
            return false;
        }
        true
    }

    /// Fence the session because the uplink disconnected.
    ///
    /// A revoked session stays revoked: revocation is the stronger statement
    /// and must not be downgraded to a disconnect that a reconnect could clear.
    pub fn fence_disconnected(&self) {
        let _ = self.fence.compare_exchange(
            FENCE_OPEN,
            FENCE_DISCONNECTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Fence the session because the enrollment was durably revoked.
    pub fn fence_revoked(&self) {
        self.fence.store(FENCE_REVOKED, Ordering::Release);
    }

    fn admit(&self) -> Result<(), ZoneLinkSessionError> {
        match self.fence.load(Ordering::Acquire) {
            FENCE_OPEN => {
                if self
                    .liveness
                    .as_ref()
                    .is_some_and(|liveness| !liveness.is_live())
                {
                    return Err(ZoneLinkSessionError::ZoneLinkDisconnected);
                }
                if let Some(admission) = &self.admission
                    && let Err(error) = admission.revalidate()
                {
                    self.fence.store(FENCE_REVOKED, Ordering::Release);
                    return Err(ZoneLinkSessionError::RouteAdmission(error));
                }
                if self.admission.as_ref().is_some_and(|admission| {
                    self.driver.generation() != admission.reconnect_generation().get()
                }) {
                    self.fence.store(FENCE_REVOKED, Ordering::Release);
                    return Err(ZoneLinkSessionError::RouteAdmission(
                        RouteAdmissionError::ReconnectGenerationMismatch,
                    ));
                }
                Ok(())
            }
            FENCE_REVOKED => Err(ZoneLinkSessionError::ZoneLinkRevoked),
            _ => Err(ZoneLinkSessionError::ZoneLinkDisconnected),
        }
    }

    /// Open a named stream for a relayed call on this hop.
    pub async fn open_forwarded_stream(
        &self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<(), ZoneLinkSessionError> {
        self.admit()?;
        self.driver
            .open_named_stream(stream, send_credit, receive_credit)
            .await
            .map_err(session_error)
    }

    /// Forward one logical message onto a relayed stream.
    pub async fn send_forwarded_stream(
        &self,
        stream: StreamId,
        bytes: Vec<u8>,
    ) -> Result<(), ZoneLinkSessionError> {
        self.admit()?;
        self.driver
            .send_named_stream(stream, bytes)
            .await
            .map_err(session_error)
    }

    /// Forward consumed credit for one relayed named stream to the next hop.
    ///
    /// The unit is logical plaintext bytes the local side actually consumed.
    /// An intermediate Zone therefore never grants credit it has not first
    /// been granted, which is what bounds a relayed stream's queue at every
    /// hop rather than only at the terminal one.
    pub async fn forward_named_stream_credit(
        &self,
        stream: StreamId,
        consumed_bytes: u32,
    ) -> Result<(), ZoneLinkSessionError> {
        self.admit()?;
        if consumed_bytes == 0 {
            // A zero grant is a no-op rather than a wakeup, so a peer cannot
            // use repeated empty grants as a liveness or timing signal.
            return Ok(());
        }
        self.driver
            .grant_named_stream_credit(stream, consumed_bytes)
            .await
            .map_err(session_error)
    }

    /// Close one relayed stream normally.
    pub async fn close_forwarded_stream(
        &self,
        stream: StreamId,
    ) -> Result<(), ZoneLinkSessionError> {
        self.admit()?;
        self.driver
            .close_named_stream(stream)
            .await
            .map_err(session_error)
    }

    /// Reset one relayed stream.
    ///
    /// A reset is admitted on a disconnected session because tearing a stream
    /// down is the correct response to a fenced link; it is still refused on a
    /// revoked one, where the whole session is gone.
    pub async fn reset_forwarded_stream(
        &self,
        stream: StreamId,
    ) -> Result<(), ZoneLinkSessionError> {
        if self.fence.load(Ordering::Acquire) == FENCE_REVOKED {
            return Err(ZoneLinkSessionError::ZoneLinkRevoked);
        }
        if self
            .liveness
            .as_ref()
            .is_some_and(|liveness| !liveness.is_live())
        {
            return Err(ZoneLinkSessionError::ZoneLinkDisconnected);
        }
        if let Some(admission) = &self.admission
            && let Err(error) = admission.revalidate()
        {
            self.fence.store(FENCE_REVOKED, Ordering::Release);
            return Err(ZoneLinkSessionError::RouteAdmission(error));
        }
        if self.admission.as_ref().is_some_and(|admission| {
            self.driver.generation() != admission.reconnect_generation().get()
        }) {
            self.fence.store(FENCE_REVOKED, Ordering::Release);
            return Err(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::ReconnectGenerationMismatch,
            ));
        }
        self.driver
            .reset_named_stream(stream)
            .await
            .map_err(session_error)
    }

    /// Deliver a cancel for one forwarded operation.
    ///
    /// The cancel is bound to this session's own reconnect generation, read
    /// from the drive loop at delivery time rather than captured earlier, so a
    /// cancel raised before a reconnect cannot be applied to the session that
    /// replaced it. Delivery is best-effort by contract and carries no
    /// deadline: a failed delivery never extends the caller's.
    pub async fn deliver_cancel(&self, request_id: RequestId) -> Result<(), ZoneLinkSessionError> {
        self.admit()?;
        let generation = self.driver.generation();
        self.driver
            .cancel(generation, request_id)
            .await
            .map_err(session_error)
    }

    /// Register an authenticated inbound forwarded call.
    pub async fn register_forwarded_call(
        &self,
        request_id: RequestId,
    ) -> Result<Cancellation, ZoneLinkSessionError> {
        self.admit()?;
        self.driver
            .register_inbound_call(request_id)
            .await
            .map_err(session_error)
    }

    /// Close the session.
    ///
    /// Fences first, so no further traffic races the close.
    pub async fn close(
        &self,
        reason: CloseReason,
        remediation: Remediation,
    ) -> Result<(), ZoneLinkSessionError> {
        self.fence_disconnected();
        self.driver
            .close(reason, remediation)
            .await
            .map_err(session_error)
    }
}

fn session_error(error: d2b_session::SessionError) -> ZoneLinkSessionError {
    ZoneLinkSessionError::Session(error.code())
}

fn route_admission_error(error: RouteAdmissionError) -> ZoneLinkSessionError {
    ZoneLinkSessionError::RouteAdmission(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::contract::{
        RouteAdmissionVerifier, RuntimeRouteAdmissionAuthority, ZoneLinkRouteAdmissionRequest,
    };
    use crate::session::enrollment::{
        BOOTSTRAP_PSK_TTL_MS_DEFAULT, BootstrapPskIssuance, EnrollmentFingerprint, EnrollmentRecord,
    };
    use async_trait::async_trait;
    use d2b_contracts_resource::v3::{
        ResourceRef, ResourceUid, ZoneId, ZoneRevision,
        identity::{AuthenticatedSubjectContext, BindingDigest, EvidenceClass, SessionBinding},
    };
    use d2b_contracts_zone_session::v3::component_session::{
        AuthorizationLease, EndpointPolicy, OperationClass, OperationId,
    };
    use d2b_contracts_zone_session::v3::zone_routing::{
        ZoneLabelId, ZoneLinkControllerGeneration, ZonePath, ZoneRouteCapability, ZoneTreeEdge,
    };
    use d2b_session::{
        AuthenticatedComponentSession, HandshakeCredentials, OwnedAttachment, OwnedTransport,
        RequestRegistry, Result as SessionResult, SessionAcceptor, SessionAuthenticationBinding,
        SessionAuthorizationRequest, SessionEngine, SessionError, SessionEvent, StreamEvent,
        TransportDescriptor, TransportEvidence, TransportPacket, TransportReader, TransportWriter,
        serialized_transport_split, x25519_public_key,
    };
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;
    use tokio::sync::mpsc;

    /// What the fake driver was asked to do, in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Open(u32, u32),
        Send(usize),
        Grant(u32),
        Close,
        Reset,
        Cancel(u64),
        Register,
        CloseSession,
    }

    struct FakeDriver {
        generation: u64,
        calls: Mutex<Vec<Call>>,
        registry: Mutex<RequestRegistry>,
    }

    impl FakeDriver {
        fn new(generation: u64) -> Self {
            Self {
                generation,
                calls: Mutex::new(Vec::new()),
                registry: Mutex::new(
                    RequestRegistry::new(generation).expect("a nonzero generation"),
                ),
            }
        }

        fn record(&self, call: Call) {
            self.calls.lock().expect("the call log").push(call);
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("the call log").clone()
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeDriver {
        fn generation(&self) -> u64 {
            self.generation
        }

        async fn start_ttrpc(&self, _request_id: RequestId, _frame: Vec<u8>) -> SessionResult<()> {
            unimplemented(())
        }

        async fn complete_ttrpc(&self, _request_id: RequestId) -> SessionResult<bool> {
            unimplemented(false)
        }

        async fn cancel(&self, generation: u64, _request_id: RequestId) -> SessionResult<()> {
            self.record(Call::Cancel(generation));
            Ok(())
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> SessionResult<()> {
            unimplemented(())
        }

        async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>> {
            unimplemented(Vec::new())
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> SessionResult<Cancellation> {
            self.record(Call::Register);
            self.registry
                .lock()
                .expect("the request registry")
                .register(request_id)
        }

        async fn mark_inbound_dispatched(&self, _request_id: RequestId) -> SessionResult<()> {
            unimplemented(())
        }

        async fn complete_inbound_call(&self, _request_id: RequestId) -> SessionResult<bool> {
            unimplemented(false)
        }

        async fn remove_inbound_call(&self, _request_id: RequestId) -> SessionResult<bool> {
            unimplemented(false)
        }

        async fn send_attachments(&self, _attachments: Vec<OwnedAttachment>) -> SessionResult<()> {
            unimplemented(())
        }

        async fn receive_attachments(&self) -> SessionResult<Vec<OwnedAttachment>> {
            unimplemented(Vec::new())
        }

        async fn open_named_stream(
            &self,
            _stream: StreamId,
            send_credit: u32,
            receive_credit: u32,
        ) -> SessionResult<()> {
            self.record(Call::Open(send_credit, receive_credit));
            Ok(())
        }

        async fn send_named_stream(&self, _stream: StreamId, bytes: Vec<u8>) -> SessionResult<()> {
            self.record(Call::Send(bytes.len()));
            Ok(())
        }

        async fn receive_named_stream(&self) -> SessionResult<StreamEvent> {
            Err(SessionError::new(SessionErrorCode::InternalInvariant))
        }

        async fn grant_named_stream_credit(
            &self,
            _stream: StreamId,
            bytes: u32,
        ) -> SessionResult<()> {
            self.record(Call::Grant(bytes));
            Ok(())
        }

        async fn close_named_stream(&self, _stream: StreamId) -> SessionResult<()> {
            self.record(Call::Close);
            Ok(())
        }

        async fn reset_named_stream(&self, _stream: StreamId) -> SessionResult<()> {
            self.record(Call::Reset);
            Ok(())
        }

        async fn drive_keepalive(&self, _now: Instant) -> SessionResult<()> {
            unimplemented(())
        }

        async fn receive_control(&self) -> SessionResult<SessionEvent> {
            Err(SessionError::new(SessionErrorCode::InternalInvariant))
        }

        async fn close(
            &self,
            _reason: CloseReason,
            _remediation: Remediation,
        ) -> SessionResult<()> {
            self.record(Call::CloseSession);
            Ok(())
        }
    }

    fn unimplemented<T>(_value: T) -> SessionResult<T> {
        Err(SessionError::new(SessionErrorCode::InternalInvariant))
    }

    struct RouteTestTransport {
        sender: mpsc::Sender<TransportPacket>,
        receiver: mpsc::Receiver<TransportPacket>,
        descriptor: TransportDescriptor,
    }

    #[async_trait]
    impl OwnedTransport for RouteTestTransport {
        fn descriptor(&self) -> TransportDescriptor {
            self.descriptor
        }

        fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
            serialized_transport_split(self)
        }

        async fn receive(
            &mut self,
            protected_limit: usize,
        ) -> Result<TransportPacket, d2b_session::TransportError> {
            let packet = self
                .receiver
                .recv()
                .await
                .ok_or(d2b_session::TransportError::Disconnected)?;
            if packet.as_bytes().len() > protected_limit {
                return Err(d2b_session::TransportError::LimitExceeded);
            }
            Ok(packet)
        }

        async fn send(
            &mut self,
            packet: TransportPacket,
        ) -> Result<(), d2b_session::TransportError> {
            self.sender
                .send(packet)
                .await
                .map_err(|_| d2b_session::TransportError::Disconnected)
        }

        async fn close(&mut self) -> Result<(), d2b_session::TransportError> {
            Ok(())
        }
    }

    fn route_test_transport_pair(
        policy: &EndpointPolicy,
    ) -> (RouteTestTransport, RouteTestTransport) {
        let (left_sender, left_receiver) = mpsc::channel(16);
        let (right_sender, right_receiver) = mpsc::channel(16);
        let descriptor = TransportDescriptor {
            class: policy.transport_binding.transport,
            locality: policy.transport_binding.locality,
            packet_atomic: false,
            supports_attachments: false,
        };
        (
            RouteTestTransport {
                sender: left_sender,
                receiver: right_receiver,
                descriptor,
            },
            RouteTestTransport {
                sender: right_sender,
                receiver: left_receiver,
                descriptor,
            },
        )
    }

    async fn authenticated_route(
        generation: u64,
    ) -> (
        AuthenticatedComponentSession<()>,
        VerifiedRouteAdmission,
        RouteAdmissionVerifier,
    ) {
        let policy = crate::session::contract::fixtures::enrolled_zone_link(generation);
        let wire_policy = policy.lower().expect("lower the enrolled ZoneLink policy");
        let parent_private = [2_u8; 32];
        let guest_private = [3_u8; 32];
        let parent_public = x25519_public_key(&parent_private).expect("parent public key");
        let guest_public = x25519_public_key(&guest_private).expect("guest public key");
        let (initiator_transport, responder_transport) = route_test_transport_pair(&wire_policy);
        let (initiator, responder) = tokio::join!(
            SessionEngine::establish_initiator(
                initiator_transport,
                wire_policy.clone(),
                HandshakeCredentials::Kk {
                    local_private: d2b_session::Secret32::new(parent_private)
                        .expect("parent private key"),
                    remote_public: guest_public,
                },
                Instant::now(),
            ),
            SessionEngine::establish_responder(
                responder_transport,
                wire_policy.clone(),
                HandshakeCredentials::Kk {
                    local_private: d2b_session::Secret32::new(guest_private)
                        .expect("guest private key"),
                    remote_public: parent_public,
                },
                Instant::now(),
            ),
        );
        let zone = ZoneId::parse("work").expect("test Zone");
        let acceptor = SessionAcceptor::from_verified_adapter(
            wire_policy.clone(),
            zone.clone(),
            move |evidence: TransportEvidence,
                  binding: &SessionAuthenticationBinding,
                  expected_zone: &ZoneId,
                  now_tick| {
                if evidence.class() != EvidenceClass::EnrolledKk || expected_zone != &zone {
                    return Err(SessionError::new(SessionErrorCode::PolicyDenied));
                }
                let subject_ref = ResourceRef::parse("Provider/zone-link").expect("subject ref");
                let subject_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                    .expect("subject uid");
                let zone_ref = ResourceRef::parse("Zone/work").expect("Zone ref");
                let session = SessionBinding::new(
                    binding.schema_fingerprint().clone(),
                    binding.transport_binding().clone(),
                    binding.reconnect_generation(),
                    binding.transcript_hash().clone(),
                );
                let subject = AuthenticatedSubjectContext::new(
                    subject_ref,
                    subject_uid,
                    zone_ref,
                    binding.evidence_class(),
                    binding.purpose().clone(),
                    binding.service().clone(),
                    session,
                );
                let lease = AuthorizationLease::new(1, now_tick.saturating_add(100))
                    .expect("session lease");
                Ok((subject, lease))
            },
            move |_subject: &AuthenticatedSubjectContext,
                  _request: &SessionAuthorizationRequest,
                  previous,
                  now_tick| {
                if previous.is_valid_at(now_tick) {
                    Ok(previous)
                } else {
                    Err(SessionError::new(SessionErrorCode::PolicyDenied))
                }
            },
            (),
        )
        .expect("session acceptor");
        let session = acceptor
            .admit(
                initiator.expect("initiator handshake"),
                TransportEvidence::new(
                    EvidenceClass::EnrolledKk,
                    BindingDigest::parse(format!("sha256:{}", "22".repeat(32)))
                        .expect("binding digest"),
                ),
                1,
            )
            .await
            .expect("authenticated route");
        let _ = responder.expect("responder handshake");
        let route = session.route_binding();
        let clock = Arc::new(AtomicU64::new(1_000));
        let clock_for_config = Arc::clone(&clock);
        let authority = RuntimeRouteAdmissionAuthority::new(
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("link UID"),
            ZoneTreeEdge::new(
                ZonePath::new(vec![ZoneLabelId::parse("parent").expect("Zone label")])
                    .expect("source path"),
                ZonePath::new(vec![
                    ZoneLabelId::parse("child").expect("Zone label"),
                    ZoneLabelId::parse("parent").expect("Zone label"),
                ])
                .expect("target path"),
            )
            .expect("Zone edge"),
            ZoneLinkControllerGeneration::parse("generation-1").expect("controller generation"),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("source Zone UID"),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").expect("target Zone UID"),
            ZoneRouteCapability::parse("resource-read").expect("capability"),
            OperationClass::Invoke,
            ZoneRevision::new(9),
            &wire_policy,
            &route,
            Arc::new(move || clock_for_config.load(Ordering::Acquire)),
        )
        .expect("route admission config");
        let request = ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![0x11; 16]).expect("operation ID"),
            OperationClass::Invoke,
        )
        .expect("route operation");
        let (verifier, evidence) = authority.issue(request).expect("issue route admission");
        let admission = verifier
            .verify(evidence)
            .expect("verify route admission");
        (session, admission, verifier)
    }

    fn ready_link() -> ZoneLinkEnrollment {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        link.begin_bootstrap(
            BootstrapPskIssuance::new(1, 0, BOOTSTRAP_PSK_TTL_MS_DEFAULT).expect("an issuance"),
            0,
        )
        .expect("bootstrap");
        link.commit_enrollment(EnrollmentRecord::new(
            EnrollmentFingerprint::new([0x11; 32]).expect("a digest"),
            [0xAB; 32],
        ))
        .expect("seal");
        link.begin_enrolled_handshake().expect("kk");
        link.establish(EnrollmentFingerprint::new([0x11; 32]).expect("a digest"), 0)
            .expect("establish");
        link
    }

    fn session(driver: Arc<FakeDriver>) -> ZoneLinkSession {
        ZoneLinkSession::establish_for_tests(&ready_link(), driver).expect("establish a session")
    }

    fn request_id() -> RequestId {
        RequestId::new(vec![0x01; 16]).expect("a bounded request id")
    }

    fn stream_id() -> StreamId {
        StreamId::new(0x0100).expect("a named-stream channel")
    }

    #[tokio::test]
    async fn authenticated_driver_lane_binds_the_exact_zone_link_profile() {
        let (authenticated, admission, _verifier) = authenticated_route(3).await;
        let session =
            ZoneLinkSession::establish_authenticated(&ready_link(), admission, authenticated)
                .expect("establish an authenticated driver lane");

        assert_eq!(session.generation(), 3);
        assert!(session.is_open());
    }

    #[tokio::test]
    async fn stale_or_revoked_route_admission_fences_the_driver_lane() {
        let (authenticated, admission, verifier) = authenticated_route(3).await;
        let session =
            ZoneLinkSession::establish_authenticated(&ready_link(), admission, authenticated)
                .expect("establish an authenticated driver lane");
        verifier.revoke();
        assert_eq!(
            session.open_forwarded_stream(stream_id(), 1, 1).await.err(),
            Some(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::Revoked
            ))
        );
        assert!(!session.is_open());
        assert_eq!(
            session.reset_forwarded_stream(stream_id()).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkRevoked)
        );
    }

    #[tokio::test]
    async fn route_admission_cannot_be_substituted_across_authenticated_sessions() {
        let (original, admission, _verifier) = authenticated_route(3).await;
        let (substitute, _other_admission, _other_verifier) = authenticated_route(3).await;
        let substitute_route = substitute.route_binding();
        assert_eq!(
            ZoneLinkSession::establish_authenticated(&ready_link(), admission, substitute).err(),
            Some(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::SessionBindingMismatch
            ))
        );
        assert!(!substitute_route.liveness().is_live());
        drop(original);
    }

    #[tokio::test]
    async fn current_policy_revision_is_rechecked_before_driver_use() {
        let (authenticated, admission, verifier) = authenticated_route(3).await;
        let session =
            ZoneLinkSession::establish_authenticated(&ready_link(), admission, authenticated)
                .expect("establish an authenticated driver lane");
        verifier
            .update_policy(
                ZoneRouteCapability::parse("resource-write").expect("capability"),
                OperationClass::Invoke,
                ZoneRevision::new(10),
            )
            .expect("advance route policy");
        assert_eq!(
            session.open_forwarded_stream(stream_id(), 1, 1).await.err(),
            Some(ZoneLinkSessionError::RouteAdmission(
                RouteAdmissionError::CapabilityMismatch
            ))
        );
        assert!(!session.is_open());
    }

    #[tokio::test]
    async fn unready_enrollment_rejects_a_verified_route_before_driver_admission() {
        let (authenticated, admission, _verifier) = authenticated_route(3).await;
        assert_eq!(
            ZoneLinkSession::establish_authenticated(
                &ZoneLinkEnrollment::new_unenrolled(),
                admission,
                authenticated,
            )
            .err(),
            Some(ZoneLinkSessionError::ResourceTrafficBeforeReady)
        );
    }

    #[test]
    fn a_session_cannot_be_established_before_the_link_is_ready() {
        let driver = Arc::new(FakeDriver::new(3));
        for link in [
            ZoneLinkEnrollment::new_unenrolled(),
            ZoneLinkEnrollment::recover(
                Some(EnrollmentRecord::new(
                    EnrollmentFingerprint::new([0x11; 32]).expect("a digest"),
                    [0xAB; 32],
                )),
                false,
                1,
            ),
        ] {
            assert_eq!(
                ZoneLinkSession::establish_for_tests(&link, driver.clone()).err(),
                Some(ZoneLinkSessionError::ResourceTrafficBeforeReady)
            );
        }
    }

    #[tokio::test]
    async fn per_hop_credit_is_forwarded_in_consumed_plaintext_bytes() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver.clone());
        session
            .forward_named_stream_credit(stream_id(), 4_096)
            .await
            .expect("forward credit");
        assert_eq!(driver.calls(), vec![Call::Grant(4_096)]);
    }

    #[tokio::test]
    async fn a_zero_credit_grant_reaches_no_peer() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver.clone());
        session
            .forward_named_stream_credit(stream_id(), 0)
            .await
            .expect("a zero grant is a no-op");
        assert!(driver.calls().is_empty());
    }

    #[tokio::test]
    async fn a_cancel_is_delivered_under_the_sessions_own_generation() {
        let driver = Arc::new(FakeDriver::new(11));
        let session = session(driver.clone());
        session
            .deliver_cancel(request_id())
            .await
            .expect("deliver the cancel");
        assert_eq!(driver.calls(), vec![Call::Cancel(11)]);
    }

    #[tokio::test]
    async fn a_disconnected_session_refuses_traffic_immediately() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver.clone());
        session.fence_disconnected();
        assert!(!session.is_open());

        assert_eq!(
            session.open_forwarded_stream(stream_id(), 1, 1).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkDisconnected)
        );
        assert_eq!(
            session
                .send_forwarded_stream(stream_id(), vec![1])
                .await
                .err(),
            Some(ZoneLinkSessionError::ZoneLinkDisconnected)
        );
        assert_eq!(
            session
                .forward_named_stream_credit(stream_id(), 8)
                .await
                .err(),
            Some(ZoneLinkSessionError::ZoneLinkDisconnected)
        );
        assert_eq!(
            session.deliver_cancel(request_id()).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkDisconnected)
        );
        assert_eq!(
            session.register_forwarded_call(request_id()).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkDisconnected)
        );
        assert!(driver.calls().is_empty());

        // Tearing a stream down is still correct on a disconnected link.
        session
            .reset_forwarded_stream(stream_id())
            .await
            .expect("reset");
        assert_eq!(driver.calls(), vec![Call::Reset]);
    }

    #[tokio::test]
    async fn a_revoked_session_refuses_everything_including_reset() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver.clone());
        session.fence_revoked();
        assert_eq!(
            session
                .forward_named_stream_credit(stream_id(), 8)
                .await
                .err(),
            Some(ZoneLinkSessionError::ZoneLinkRevoked)
        );
        assert_eq!(
            session.reset_forwarded_stream(stream_id()).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkRevoked)
        );
        assert!(driver.calls().is_empty());
    }

    #[tokio::test]
    async fn revocation_is_not_downgraded_by_a_later_disconnect() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver);
        session.fence_revoked();
        session.fence_disconnected();
        assert_eq!(
            session.deliver_cancel(request_id()).await.err(),
            Some(ZoneLinkSessionError::ZoneLinkRevoked)
        );
    }

    #[tokio::test]
    async fn the_happy_path_forwards_open_send_credit_and_close_in_order() {
        let driver = Arc::new(FakeDriver::new(5));
        let session = session(driver.clone());
        session
            .open_forwarded_stream(stream_id(), 64, 64)
            .await
            .expect("open");
        session
            .send_forwarded_stream(stream_id(), vec![0; 12])
            .await
            .expect("send");
        session
            .forward_named_stream_credit(stream_id(), 12)
            .await
            .expect("credit");
        session
            .close_forwarded_stream(stream_id())
            .await
            .expect("close");
        assert_eq!(
            driver.calls(),
            vec![
                Call::Open(64, 64),
                Call::Send(12),
                Call::Grant(12),
                Call::Close
            ]
        );
    }

    #[tokio::test]
    async fn closing_the_session_fences_it_first() {
        let driver = Arc::new(FakeDriver::new(3));
        let session = session(driver.clone());
        session
            .close(CloseReason::Normal, Remediation::None)
            .await
            .expect("close");
        assert!(!session.is_open());
        assert_eq!(driver.calls(), vec![Call::CloseSession]);
    }

    #[test]
    fn the_session_reports_its_link_epoch_and_generation() {
        let driver = Arc::new(FakeDriver::new(42));
        let session = session(driver);
        assert_eq!(session.epoch(), LinkEpoch::FIRST);
        assert_eq!(session.generation(), 42);
    }

    #[test]
    fn debug_output_names_no_driver_stream_or_generation() {
        let driver = Arc::new(FakeDriver::new(42));
        let rendered = format!("{:?}", session(driver));
        assert!(rendered.starts_with("ZoneLinkSession {"));
        assert!(!rendered.contains("42"));
    }

    #[test]
    fn every_refusal_renders_a_closed_path_free_label() {
        for error in [
            ZoneLinkSessionError::ResourceTrafficBeforeReady,
            ZoneLinkSessionError::ZoneLinkDisconnected,
            ZoneLinkSessionError::ZoneLinkRevoked,
            ZoneLinkSessionError::Session(SessionErrorCode::Cancelled),
        ] {
            assert_eq!(error.as_str(), format!("{error}"));
            assert!(!error.as_str().contains('/'));
        }
    }
}
