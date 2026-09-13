//! The Guest agent lifecycle base.
//!
//! A Guest agent implements [`GuestAgent`] and nothing else runs in its
//! `main`: the toolkit owns the sequence below it, so two Guest agents cannot
//! diverge on link connect, enrollment, the service loop, reconnect, or drain
//! ordering.
//!
//! ```text
//! link connect -> zone-bootstrap -> zone-enroll -> service loop -> drain
//! ```
//!
//! What stays with the agent is the type-specific surface: the frames it
//! serves and the events it raises on its own initiative. What stays with the
//! Guest's own crate is the transport: the base opens no socket, resolves no
//! address, and holds no credential, so a Guest binary is free to carry the
//! session over the contract's native-vsock framing while a test carries it
//! over an in-memory duplex.
//!
//! # Enrollment is the allocator's, not the agent's
//!
//! A Guest agent is placed by the allocator: it cannot name its own Zone, mint
//! its own enrollment, or choose the ZoneLink it joins. The base drives the
//! two landed Zone service methods - `zone-bootstrap`, which consumes the
//! allocator-issued single-use PSK, and `zone-enroll`, which seals the
//! enrollment record and admits the peer - and answers with the allocator's
//! own tuple: the placed `ZoneId` and the enrollment generation. Nothing in
//! this module accepts or returns a uid, gid, host path, socket path, store
//! path, credential, or key byte, and no host `Principal` row crosses the
//! trust boundary.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_contracts_zone_session::v3::zone_session::{
    ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall, ZoneEnrollReply, ZoneEnrollmentIdentity,
    ZoneEnrollmentRefusal,
};
use d2b_resource_types::DriverDescriptor;
use d2b_session::{OwnedTransport, TransportPacket};

use crate::declaration::ProviderDeclaration;
use crate::plane::DrainDeadline;

use super::{DEFAULT_DRAIN_BUDGET_MS, DrainError, SystemClock};

/// Largest session frame this base accepts from or sends to the allocator.
///
/// The bound is enforced on both sides of every exchange, so a peer cannot
/// make a Guest agent allocate an unbounded buffer.
pub const GUEST_SESSION_MAX_FRAME_BYTES: usize = 64 * 1024;

/// Bounded connect attempts before a Guest agent gives up on its allocator.
pub const GUEST_RECONNECT_ATTEMPTS: u32 = 8;

/// First reconnect wait, in milliseconds.
pub const GUEST_RECONNECT_INITIAL_MS: u64 = 1_000;

/// Longest reconnect wait, in milliseconds.
pub const GUEST_RECONNECT_MAX_MS: u64 = 60_000;

/// Why a Guest agent could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestError {
    /// The Guest link could not be opened within its bounded attempts.
    LinkUnavailable,
    /// The allocator refused the enrollment for one closed reason.
    EnrollmentRefused(ZoneEnrollmentRefusal),
    /// The allocator's answer was truncated, oversized, or not a reply this
    /// base can act on.
    EnrollmentMalformed,
    /// The enrolled session ended before the agent finished serving.
    SessionDisconnected,
    /// The agent refused one frame.
    ServeRefused,
    /// The agent failed to drain within its deadline.
    Refused,
}

impl GuestError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::LinkUnavailable => "guest-link-unavailable",
            Self::EnrollmentRefused(_) => "guest-enrollment-refused",
            Self::EnrollmentMalformed => "guest-enrollment-malformed",
            Self::SessionDisconnected => "guest-session-disconnected",
            Self::ServeRefused => "guest-serve-refused",
            Self::Refused => "guest-refused",
        }
    }

    /// The closed enrollment refusal, when the allocator refused one.
    pub const fn enrollment_refusal(&self) -> Option<ZoneEnrollmentRefusal> {
        match self {
            Self::EnrollmentRefused(reason) => Some(*reason),
            _ => None,
        }
    }
}

impl std::fmt::Display for GuestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GuestError {}

/// One bounded frame carried by an enrolled Guest session.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestFrame {
    payload: Vec<u8>,
}

impl GuestFrame {
    /// Build one frame, refusing an empty or oversized payload.
    pub fn new(payload: Vec<u8>) -> Result<Self, GuestError> {
        if payload.is_empty() || payload.len() > GUEST_SESSION_MAX_FRAME_BYTES {
            return Err(GuestError::ServeRefused);
        }
        Ok(Self { payload })
    }

    /// Borrow the frame payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.payload
    }

    /// Consume the frame, returning its payload.
    pub fn into_bytes(self) -> Vec<u8> {
        self.payload
    }
}

impl std::fmt::Debug for GuestFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestFrame")
            .field("bytes", &self.payload.len())
            .finish()
    }
}

/// Boxed future returned by [`GuestLink::connect`].
pub type GuestLinkFuture =
    Pin<Box<dyn Future<Output = Result<Box<dyn OwnedTransport>, GuestError>> + Send>>;

/// The one transport a Guest agent session is carried over.
///
/// The base never constructs a transport itself: the agent's own crate supplies
/// the connect step, so the same agent runs over native vsock in the Guest and
/// over an in-memory duplex in a test without the base knowing either.
pub trait GuestLink: Send + Sync + 'static {
    /// Open one connection to the allocator.
    fn connect(&self) -> GuestLinkFuture;
}

/// The placement facts one Guest agent was started with.
///
/// These are the allocator's own facts, handed to the agent when it was
/// placed: the link identity, the single-use PSK issuance the allocator minted
/// for this generation, and the Guest's own static-key fingerprint, which the
/// allocator pinned. No PSK byte and no static key byte appears here - an
/// issuance is its ordinal and lifetime, and a fingerprint is a digest.
pub struct GuestPlacement {
    identity: ZoneEnrollmentIdentity,
    psk_issuance: u64,
    psk_ttl_ms: u64,
    psk_issued_at_unix_ms: u64,
    static_key_fingerprint: [u8; 32],
}

impl GuestPlacement {
    /// Bind one placement.
    pub fn new(
        identity: ZoneEnrollmentIdentity,
        psk_issuance: u64,
        psk_ttl_ms: u64,
        psk_issued_at_unix_ms: u64,
        static_key_fingerprint: [u8; 32],
    ) -> Result<Self, GuestError> {
        if psk_issuance == 0
            || psk_ttl_ms == 0
            || static_key_fingerprint == [0; 32]
            || identity.schema_fingerprint == [0; 32]
        {
            return Err(GuestError::EnrollmentMalformed);
        }
        Ok(Self {
            identity,
            psk_issuance,
            psk_ttl_ms,
            psk_issued_at_unix_ms,
            static_key_fingerprint,
        })
    }

    /// The link identity this placement is for.
    pub const fn identity(&self) -> &ZoneEnrollmentIdentity {
        &self.identity
    }
}

impl std::fmt::Debug for GuestPlacement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GuestPlacement(<redacted>)")
    }
}

/// A Guest agent: the base's shape minus every host-plane bit.
#[async_trait]
pub trait GuestAgent: Send + Sync + 'static {
    /// The identity and non-host facts this agent declares.
    fn declaration(&self) -> &ProviderDeclaration;

    /// What this agent serves, one descriptor per resource type.
    fn drivers(&self) -> &[DriverDescriptor];

    /// Serve one admitted frame from the enrolled session.
    ///
    /// The agent returns the frames to write back, in order; an empty answer
    /// is a served request with nothing to say. A refusal is the agent's, and
    /// the session stays up: only the frame is dropped.
    async fn serve(&self, frame: GuestFrame) -> Result<Vec<GuestFrame>, GuestError>;

    /// Await the next frame this agent raises on its own initiative.
    ///
    /// MUST be cancel-safe: the base cancels this future whenever an inbound
    /// frame arrives first. `None` means this agent raises no further events.
    async fn next_event(&self) -> Option<GuestFrame> {
        std::future::pending().await
    }

    /// Tear down within the deadline.
    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError>;
}

/// The enrolled route a Guest agent serves on.
///
/// This is the allocator's own answer: the Zone it placed the agent in and the
/// link epoch the enrolled session was assigned. It carries no authority, no
/// session handle, and no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrolledRoute {
    /// The Zone the enrolled session is scoped to.
    pub zone: ZoneId,
    /// The generation the enrollment issued.
    pub generation: u64,
}

/// The guest enrollment seam.
///
/// One implementation is the production path; a test may substitute its own.
#[async_trait]
pub trait GuestEnrollment: Send + Sync {
    /// Enroll the Guest agent over one open link.
    async fn enroll(
        &self,
        link: &mut dyn OwnedTransport,
        request: &EnrollmentRequest,
    ) -> Result<EnrolledRoute, GuestError>;
}

/// One guest enrollment request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentRequest {
    /// The `Provider/<name>` identity the Guest agent serves as.
    ///
    /// The zone arrives with the enrollment: a Guest agent is placed by the
    /// allocator it enrolls with, so it cannot name its own zone first.
    pub provider_ref: ResourceRef,
}

/// The allocator enrollment: the one production [`GuestEnrollment`].
///
/// It drives the two landed Zone service methods over the open link and
/// nothing else. The bootstrap presents the allocator's issuance descriptor
/// and burns it; the enrollment presents the identity and the peer's own
/// observed static-key fingerprint. Every refusal the allocator answers with
/// is reported as its own closed reason rather than collapsed into a generic
/// failure.
pub struct AllocatorEnrollment {
    placement: GuestPlacement,
}

impl AllocatorEnrollment {
    /// Bind the enrollment to one placement.
    pub const fn new(placement: GuestPlacement) -> Self {
        Self { placement }
    }
}

impl std::fmt::Debug for AllocatorEnrollment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AllocatorEnrollment(<redacted>)")
    }
}

#[async_trait]
impl GuestEnrollment for AllocatorEnrollment {
    async fn enroll(
        &self,
        link: &mut dyn OwnedTransport,
        _request: &EnrollmentRequest,
    ) -> Result<EnrolledRoute, GuestError> {
        let bootstrap = ZoneBootstrapCall::new(
            self.placement.identity.clone(),
            self.placement.psk_issuance,
            self.placement.psk_ttl_ms,
            self.placement.psk_issued_at_unix_ms,
        );
        let payload = bootstrap
            .encode()
            .map_err(|_| GuestError::EnrollmentMalformed)?;
        match exchange(link, &payload).await? {
            EnrollmentReply::Bootstrap(ZoneBootstrapReply::Admitted { .. }) => {}
            EnrollmentReply::Bootstrap(ZoneBootstrapReply::Refused { reason }) => {
                return Err(GuestError::EnrollmentRefused(reason));
            }
            // An enroll answer to a bootstrap call is not a reply this base
            // can act on, so the session is abandoned rather than guessed at.
            EnrollmentReply::Enroll(_) => return Err(GuestError::EnrollmentMalformed),
        }

        let enroll = ZoneEnrollCall::new(
            self.placement.identity.clone(),
            self.placement.static_key_fingerprint,
            now_unix_ms(),
        );
        let payload = enroll.encode().map_err(|_| GuestError::EnrollmentMalformed)?;
        match exchange(link, &payload).await? {
            EnrollmentReply::Enroll(ZoneEnrollReply::Enrolled { zone, generation }) => {
                Ok(EnrolledRoute { zone, generation })
            }
            EnrollmentReply::Enroll(ZoneEnrollReply::Refused { reason }) => {
                Err(GuestError::EnrollmentRefused(reason))
            }
            EnrollmentReply::Bootstrap(_) => Err(GuestError::EnrollmentMalformed),
        }
    }
}

enum EnrollmentReply {
    Bootstrap(ZoneBootstrapReply),
    Enroll(ZoneEnrollReply),
}

/// Send one enrollment frame and read its bounded answer.
async fn exchange(
    link: &mut dyn OwnedTransport,
    payload: &[u8],
) -> Result<EnrollmentReply, GuestError> {
    link.send(TransportPacket::new(payload.to_vec()))
        .await
        .map_err(|_| GuestError::SessionDisconnected)?;
    let packet = link
        .receive(GUEST_SESSION_MAX_FRAME_BYTES)
        .await
        .map_err(|_| GuestError::SessionDisconnected)?;
    let (bytes, attachments) = packet.into_parts();
    if !attachments.is_empty() {
        return Err(GuestError::EnrollmentMalformed);
    }
    if let Ok(reply) = ZoneBootstrapReply::decode(&bytes) {
        return Ok(EnrollmentReply::Bootstrap(reply));
    }
    ZoneEnrollReply::decode(&bytes)
        .map(EnrollmentReply::Enroll)
        .map_err(|_| GuestError::EnrollmentMalformed)
}

/// Milliseconds since the Unix epoch, or `None` when the clock is before it.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Run one Guest agent on the base, in the Guest trust domain.
///
/// The process runs the lifecycle above and exits with the agent's status
/// code: `0` once the enrolled session and its drain completed, `1` otherwise.
pub fn run_guest<A: GuestAgent>(
    agent: A,
    link: Box<dyn GuestLink>,
    enrollment: Arc<dyn GuestEnrollment>,
) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return 1,
    };
    match runtime.block_on(serve_guest(agent, link, enrollment)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Run the Guest lifecycle: connect, enroll, serve, drain.
async fn serve_guest<A: GuestAgent>(
    agent: A,
    link: Box<dyn GuestLink>,
    enrollment: Arc<dyn GuestEnrollment>,
) -> Result<(), GuestError> {
    let provider_ref =
        ResourceRef::parse(&format!("Provider/{}", agent.declaration().provider_ref))
            .map_err(|_| GuestError::EnrollmentRefused(ZoneEnrollmentRefusal::PolicyDenial))?;
    let request = EnrollmentRequest { provider_ref };

    let mut wait_ms = GUEST_RECONNECT_INITIAL_MS;
    let mut last = GuestError::LinkUnavailable;
    for attempt in 0..GUEST_RECONNECT_ATTEMPTS {
        let mut transport = match link.connect().await {
            Ok(transport) => transport,
            Err(error) => {
                last = error;
                if attempt + 1 < GUEST_RECONNECT_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    wait_ms = (wait_ms.saturating_mul(2)).min(GUEST_RECONNECT_MAX_MS);
                }
                continue;
            }
        };
        match enrollment.enroll(transport.as_mut(), &request).await {
            Ok(route) => {
                // The enrolled route is the allocator's answer; the base uses
                // it to name the session in its own logs and for nothing else.
                tracing::debug!(generation = route.generation, "guest session enrolled");
                let served = serve_enrolled(&agent, transport).await;
                let drained = agent
                    .drain(DrainDeadline::new(Arc::new(SystemClock), DEFAULT_DRAIN_BUDGET_MS))
                    .await;
                served?;
                drained.map_err(|_| GuestError::Refused)?;
                return Ok(());
            }
            Err(error) => {
                // An enrollment the allocator refused is terminal: the
                // placement is what it is, and retrying the same issuance
                // would burn nothing but time.
                if matches!(error, GuestError::EnrollmentRefused(_)) {
                    return Err(error);
                }
                last = error;
                if attempt + 1 < GUEST_RECONNECT_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    wait_ms = (wait_ms.saturating_mul(2)).min(GUEST_RECONNECT_MAX_MS);
                }
            }
        }
    }
    Err(last)
}

/// The service loop: serve inbound frames, raise the agent's own, until the
/// session ends.
///
/// The loop owns the dispatch, so an agent cannot forget to close the writer,
/// to bound a frame, or to stop on its own signal. A frame the agent refuses
/// is dropped and the session stays up: the refusal is the agent's to record,
/// and one bad frame is not a reason to tear down an enrolled session. A
/// session the allocator closes is the normal end of serving, not a failure.
async fn serve_enrolled<A: GuestAgent>(
    agent: &A,
    transport: Box<dyn OwnedTransport>,
) -> Result<(), GuestError> {
    let (mut reader, mut writer) = transport.into_split();
    loop {
        tokio::select! {
            biased;
            packet = reader.receive(GUEST_SESSION_MAX_FRAME_BYTES) => {
                let Ok(packet) = packet else {
                    return Ok(());
                };
                let (bytes, attachments) = packet.into_parts();
                if !attachments.is_empty() {
                    return Err(GuestError::SessionDisconnected);
                }
                let Ok(frame) = GuestFrame::new(bytes) else {
                    continue;
                };
                let Ok(replies) = agent.serve(frame).await else {
                    continue;
                };
                for reply in replies {
                    writer
                        .send(TransportPacket::new(reply.into_bytes()))
                        .await
                        .map_err(|_| GuestError::SessionDisconnected)?;
                }
            }
            event = agent.next_event() => {
                let Some(event) = event else {
                    return Ok(());
                };
                writer
                    .send(TransportPacket::new(event.into_bytes()))
                    .await
                    .map_err(|_| GuestError::SessionDisconnected)?;
            }
        }
    }
}
