//! Guest-side target runtime (spec sections 23.2-23.4; R18, R19, R21).
//!
//! A Host-zone resource that targets a Guest is realized by target-local
//! effect code inside that Guest. The Guest-side bookkeeping for those
//! realizations lives here:
//!
//! - [`TargetResourceInstance`] is the spec section 23.2 target-local object.
//!   It is an implementation detail of the target runtime: it has no desired
//!   spec of its own, is never exposed through the Guest's Resource API, and
//!   never enters the Guest resource namespace as a second authoritative
//!   resource.
//! - [`GuestTargetControl`] is the target-control port the Host side reaches
//!   through the authenticated ComponentSession (spec section 23.3). The
//!   Host-side implementation of this port is the composition's
//!   session-backed client; the Guest-side implementation is
//!   [`SessionBoundGuestTargetControl`].
//! - [`SessionBoundGuestTargetControl`] is one capability bound to one
//!   authenticated ComponentSession generation. Every request it carries is
//!   fenced by that generation, so a session that reconnected at a newer
//!   generation (or an old capability retained across a reconnect) can never
//!   realize, observe, or delete on behalf of its successor.

pub const MODULE_NAME: &str = "guest_target";

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use crate::identity::ResourceKey;
use crate::target::{TargetObservation, TargetRef};

/// Target-local realization phase of one Host-zone resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetInstanceState {
    /// The target-local effect is still converging on the desired state.
    Realizing,
    /// The target-local effect is realized and serving.
    Ready,
}

impl TargetInstanceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realizing => "realizing",
            Self::Ready => "ready",
        }
    }
}

/// One target-local realization (spec section 23.2).
///
/// Keyed by the owning Host-zone resource identity: [`Self::source`] is the
/// durable key of the resource in its authority Zone, never a guest-local
/// name, so a Guest never learns a second identity for the resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResourceInstance {
    source: ResourceKey,
    source_uid: [u8; 16],
    assignment_generation: u64,
    session_generation: u64,
    local_handle: String,
    spec_digest: String,
    state: TargetInstanceState,
}

impl TargetResourceInstance {
    /// Construct the recorded shape of one realization (target-control
    /// framing only; effect code records realizations through
    /// [`GuestTargetRuntime`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: ResourceKey,
        source_uid: [u8; 16],
        assignment_generation: u64,
        session_generation: u64,
        local_handle: String,
        spec_digest: String,
        state: TargetInstanceState,
    ) -> Self {
        Self {
            source,
            source_uid,
            assignment_generation,
            session_generation,
            local_handle,
            spec_digest,
            state,
        }
    }

    /// Borrow the owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The owning resource's stable uid.
    pub const fn source_uid(&self) -> &[u8; 16] {
        &self.source_uid
    }

    /// The desired generation this realization was minted for.
    pub const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    /// The session generation that created or adopted this realization.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// The opaque target-local handle (socket path, pid, mount identity)
    /// minted by target-local effect code. It is never a Host path.
    pub fn local_handle(&self) -> &str {
        &self.local_handle
    }

    /// The commitment over the spec this realization was recorded with: the
    /// host checks it against what it sent, so a guest that recorded a
    /// different spec is refused instead of adopted.
    pub fn spec_digest(&self) -> &str {
        &self.spec_digest
    }

    /// Current target-local phase.
    pub const fn state(&self) -> TargetInstanceState {
        self.state
    }
}

/// Outcome of target-local discovery for one source (F2, F5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestAdoption {
    /// A target-local realization was present and is re-adopted under the
    /// current session generation. It is the same realization: adoption never
    /// mints a second one.
    Adopted(TargetResourceInstance),
    /// Nothing is realized on the target; the Host-owned actor realizes it.
    Missing,
}

impl GuestAdoption {
    /// The adopted realization, when discovery found one.
    pub const fn instance(&self) -> Option<&TargetResourceInstance> {
        match self {
            Self::Adopted(instance) => Some(instance),
            Self::Missing => None,
        }
    }
}

/// The target-control protocol version token (spec section 23.3).
///
/// One frame carries exactly one version token, and a Guest daemon of this
/// version answers this protocol and nothing else: the legacy guest-local
/// Resource API is not a target-control channel (R20, R29).
pub const TARGET_CONTROL_PROTOCOL: &str = "d2b.target-control.v1";

/// Domain tag the host covers with [`target_local_spec_digest`].
pub const TARGET_CONTROL_SPEC_DOMAIN: &str = "d2b-target-control-spec-v1";

/// The commitment the host attaches to a target-local spec: `sha256:<64
/// lowercase hex>` over the domain tag and the exact spec bytes the guest is
/// asked to apply. The guest recomputes it before applying anything, so a
/// substituted or truncated spec is refused instead of realized.
pub fn target_local_spec_digest(spec: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(TARGET_CONTROL_SPEC_DOMAIN.as_bytes());
    digest.update([0_u8]);
    digest.update(spec);
    let bytes = digest.finalize();
    let mut rendered = String::with_capacity(7 + 64);
    rendered.push_str("sha256:");
    for byte in bytes {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

/// The ttrpc service and method the guest serves this protocol on.
pub const TARGET_CONTROL_SERVICE: &str = "d2b.target-control.v1.TargetControl";
/// The single method of [`TARGET_CONTROL_SERVICE`].
pub const TARGET_CONTROL_METHOD: &str = "TargetControl";

/// One assignment as carried on the wire: the owning Host-zone resource, the
/// desired generation it realizes, and the session generation the request
/// belongs to.
///
/// The session generation is mandatory on every request. A request naming a
/// generation that is not the live one is answered
/// [`TargetControlResponse::SessionUnavailable`] and performs no effect -
/// the same fence the host directory enforces, on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetControlAssignment {
    source: ResourceKey,
    source_uid: [u8; 16],
    assignment_generation: u64,
    session_generation: u64,
}

impl TargetControlAssignment {
    pub fn new(
        source: ResourceKey,
        source_uid: [u8; 16],
        assignment_generation: u64,
        session_generation: u64,
    ) -> Self {
        Self { source, source_uid, assignment_generation, session_generation }
    }

    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    pub const fn source_uid(&self) -> &[u8; 16] {
        &self.source_uid
    }

    pub const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }
}

/// One target-control request (spec section 23.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetControlRequest {
    /// Realize (create or update) the target-local instance of one resource.
    Realize(GuestRealizeRequest),
    /// Observe it; the answer distinguishes absent from unavailable.
    Observe { assignment: TargetControlAssignment },
    /// Delete it; the answer reports the target-local effect, not the row.
    Delete { assignment: TargetControlAssignment },
    /// Re-run target-local discovery after a reconnect.
    Adopt { assignment: TargetControlAssignment },
}

impl TargetControlRequest {
    /// The session generation this request belongs to.
    pub const fn session_generation(&self) -> u64 {
        match self {
            Self::Realize(request) => request.session_generation(),
            Self::Observe { assignment }
            | Self::Delete { assignment }
            | Self::Adopt { assignment } => assignment.session_generation,
        }
    }

    /// The resource this request acts on.
    pub const fn source(&self) -> &ResourceKey {
        match self {
            Self::Realize(request) => request.source(),
            Self::Observe { assignment }
            | Self::Delete { assignment }
            | Self::Adopt { assignment } => &assignment.source,
        }
    }
}

/// One target-control response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetControlResponse {
    /// The realization is present on the target.
    Realized { realization: TargetResourceInstance },
    /// The observation, including `Absent` versus `Unavailable`.
    Observed(TargetObservation),
    /// The target-local effect was deleted (or was already absent).
    Deleted,
    /// Target-local discovery and adoption result.
    Adopted(GuestAdoption),
    /// The request's session generation is not the live one, or no session is
    /// live: nothing was read, written, or deleted.
    SessionUnavailable,
}

/// One framed target-control message: version token plus one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetControlFrame {
    protocol: String,
    request: TargetControlRequest,
}

impl TargetControlFrame {
    /// Frame one request with the current protocol token.
    pub fn new(request: TargetControlRequest) -> Self {
        Self { protocol: TARGET_CONTROL_PROTOCOL.to_owned(), request }
    }

    /// The protocol token carried by this frame.
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// The carried request.
    pub const fn request(&self) -> &TargetControlRequest {
        &self.request
    }

    /// Consume the frame and return its request, refusing any other protocol.
    pub fn into_request(self) -> Result<TargetControlRequest, GuestTargetError> {
        if self.protocol != TARGET_CONTROL_PROTOCOL {
            return Err(GuestTargetError::ProtocolMismatch);
        }
        Ok(self.request)
    }
}

/// One realize request across the ComponentSession target-control boundary.
///
/// It carries the assignment (owner identity, desired generation, session
/// generation), the spec the Guest must apply for the target-local
/// realization together with its commitment, and the opaque target-local
/// handle chosen by target-local effect code. The spec is authored by the
/// owning resource's driver: only the driver knows the concrete target-local
/// shape of its type, and the guest never invents one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestRealizeRequest {
    assignment: TargetControlAssignment,
    spec: Vec<u8>,
    spec_digest: String,
    local_handle: String,
}

impl GuestRealizeRequest {
    /// Construct one realize request.
    pub fn new(
        assignment: TargetControlAssignment,
        spec: Vec<u8>,
        spec_digest: impl Into<String>,
        local_handle: impl Into<String>,
    ) -> Self {
        Self {
            assignment,
            spec,
            spec_digest: spec_digest.into(),
            local_handle: local_handle.into(),
        }
    }

    /// The assignment this realization belongs to.
    pub const fn assignment(&self) -> &TargetControlAssignment {
        &self.assignment
    }

    /// Borrow the owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        self.assignment.source()
    }

    /// The owning resource's stable uid.
    pub const fn source_uid(&self) -> &[u8; 16] {
        self.assignment.source_uid()
    }

    /// The desired generation this realization is minted for.
    pub const fn assignment_generation(&self) -> u64 {
        self.assignment.assignment_generation()
    }

    /// The authenticated session generation this request belongs to.
    pub const fn session_generation(&self) -> u64 {
        self.assignment.session_generation()
    }

    /// The spec the Guest applies for this realization (the owning driver's
    /// target-local shape, never invented by the target).
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }

    /// The commitment over that spec (`sha256:<64 lowercase hex>`), computed
    /// by the host and verified by the Guest before it applies anything.
    pub fn spec_digest(&self) -> &str {
        &self.spec_digest
    }

    /// Borrow the target-local handle.
    pub fn local_handle(&self) -> &str {
        &self.local_handle
    }
}

/// The Guest-side target-control port (spec section 23.3).
///
/// One implementation serves one authenticated ComponentSession generation.
/// A stale generation is refused by the implementation, never by the caller,
/// and callers never see a target-local handle they did not send.
#[async_trait]
pub trait GuestTargetControl: Send + Sync + fmt::Debug + 'static {
    /// Realize (create or update) the target-local instance for one resource.
    /// Realization is idempotent per source: a second request for the same
    /// source updates the same instance and never mints a second one.
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError>;

    /// Observe the target-local instance for one assignment. The answer
    /// distinguishes absent from unavailable.
    async fn observe(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<TargetObservation, GuestTargetError>;

    /// Delete the target-local instance for one assignment.
    async fn delete(&self, assignment: &TargetControlAssignment) -> Result<(), GuestTargetError>;

    /// Target-local discovery and adoption for one assignment (F5). This is
    /// the only operation accepted across a generation change: it re-binds an
    /// existing realization to the current session instead of inheriting an
    /// older session's authority.
    async fn adopt(&self, assignment: &TargetControlAssignment)
    -> Result<GuestAdoption, GuestTargetError>;
}

#[derive(Debug)]
struct GuestTargetState {
    reference: TargetRef,
    session_generation: Option<u64>,
    instances: HashMap<ResourceKey, TargetResourceInstance>,
}

/// The Guest-side target runtime.
///
/// One runtime per Guest. It owns the target-local realizations of resources
/// whose authority stays in the parent Host Zone; it owns no desired spec, no
/// store, and no resource namespace.
#[derive(Debug, Clone)]
pub struct GuestTargetRuntime {
    inner: Arc<Mutex<GuestTargetState>>,
}

impl GuestTargetRuntime {
    pub fn new(reference: TargetRef) -> Self {
        Self {
            inner: Arc::new(Mutex::new(GuestTargetState {
                reference,
                session_generation: None,
                instances: HashMap::new(),
            })),
        }
    }

    /// The Guest this runtime realizes for.
    pub fn reference(&self) -> TargetRef {
        self.inner.lock().reference.clone()
    }

    /// The authenticated ComponentSession generation currently bound.
    pub fn session_generation(&self) -> Option<u64> {
        self.inner.lock().session_generation
    }

    /// Bind the authenticated session generation of the parent
    /// ComponentSession. A generation older than the live one is refused: a
    /// reconnect can never inherit an older session's authority, and binding
    /// the same generation again is a no-op.
    pub fn bind_session(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        if session_generation == 0 {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let mut state = self.inner.lock();
        if let Some(live) = state.session_generation {
            if session_generation < live {
                return Err(GuestTargetError::SessionGenerationRegression);
            }
            if session_generation == live {
                return Ok(());
            }
        }
        state.session_generation = Some(session_generation);
        Ok(())
    }

    /// Every target-local realization this runtime holds, in identity order.
    pub fn instances(&self) -> Vec<TargetResourceInstance> {
        let mut instances: Vec<TargetResourceInstance> =
            self.inner.lock().instances.values().cloned().collect();
        instances.sort_by(|left, right| identity_order(&left.source).cmp(&identity_order(&right.source)));
        instances
    }

    /// The target-local realization for one source, when present.
    pub fn instance(&self, source: &ResourceKey) -> Option<TargetResourceInstance> {
        self.inner.lock().instances.get(source).cloned()
    }

    /// A target-control capability bound to one authenticated generation.
    ///
    /// Fails closed: the generation must be the live one, so an old session
    /// cannot mint a capability for a newer one.
    pub fn control(
        self: &Arc<Self>,
        session_generation: u64,
    ) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
        SessionBoundGuestTargetControl::new(Arc::clone(self), session_generation)
    }

    fn require_generation(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        match self.inner.lock().session_generation {
            Some(live) if live == session_generation => Ok(()),
            Some(_) => Err(GuestTargetError::StaleSessionGeneration),
            None => Err(GuestTargetError::SessionUnavailable),
        }
    }

    fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError> {
        let session_generation = request.session_generation();
        self.require_generation(session_generation)?;
        let mut state = self.inner.lock();
        let instance = TargetResourceInstance {
            source: request.source().clone(),
            source_uid: *request.source_uid(),
            assignment_generation: request.assignment_generation(),
            session_generation,
            local_handle: request.local_handle().to_owned(),
            spec_digest: request.spec_digest().to_owned(),
            state: TargetInstanceState::Realizing,
        };
        state.instances.insert(request.source().clone(), instance.clone());
        Ok(instance)
    }

    /// Handle one framed target-control request: the Guest-side consumer of
    /// the target-control protocol (spec section 23.3).
    ///
    /// The fence is the request's own session generation, so a request that
    /// names a generation which is not the live one answers
    /// [`TargetControlResponse::SessionUnavailable`] and performs no effect at
    /// all - no read, no realize, no delete, no adoption.
    pub fn handle(&self, request: TargetControlRequest) -> TargetControlResponse {
        match request {
            TargetControlRequest::Realize(request) => match self.realize(request) {
                Ok(realization) => TargetControlResponse::Realized { realization },
                Err(_) => TargetControlResponse::SessionUnavailable,
            },
            TargetControlRequest::Observe { assignment } => {
                match self.observe(assignment.session_generation(), assignment.source()) {
                    Ok(None) => TargetControlResponse::Observed(TargetObservation::Absent),
                    Ok(Some(instance)) => {
                        TargetControlResponse::Observed(match instance.state() {
                            TargetInstanceState::Realizing => TargetObservation::Realizing {
                                session_generation: instance.session_generation(),
                            },
                            TargetInstanceState::Ready => TargetObservation::Ready {
                                session_generation: instance.session_generation(),
                            },
                        })
                    }
                    Err(_) => TargetControlResponse::SessionUnavailable,
                }
            }
            TargetControlRequest::Delete { assignment } => {
                match self.delete(assignment.session_generation(), assignment.source()) {
                    Ok(_) => TargetControlResponse::Deleted,
                    Err(_) => TargetControlResponse::SessionUnavailable,
                }
            }
            TargetControlRequest::Adopt { assignment } => {
                match self.adopt(assignment.session_generation(), &[assignment.source().clone()]) {
                    Ok(mut adopted) => TargetControlResponse::Adopted(
                        adopted.pop().unwrap_or(GuestAdoption::Missing),
                    ),
                    Err(_) => TargetControlResponse::SessionUnavailable,
                }
            }
        }
    }

    fn observe(
        &self,
        session_generation: u64,
        source: &ResourceKey,
    ) -> Result<Option<TargetResourceInstance>, GuestTargetError> {
        self.require_generation(session_generation)?;
        Ok(self.inner.lock().instances.get(source).cloned())
    }

    fn delete(
        &self,
        session_generation: u64,
        source: &ResourceKey,
    ) -> Result<bool, GuestTargetError> {
        self.require_generation(session_generation)?;
        Ok(self.inner.lock().instances.remove(source).is_some())
    }

    fn adopt(
        &self,
        session_generation: u64,
        sources: &[ResourceKey],
    ) -> Result<Vec<GuestAdoption>, GuestTargetError> {
        self.require_generation(session_generation)?;
        let mut state = self.inner.lock();
        let mut adopted = Vec::with_capacity(sources.len());
        for source in sources {
            match state.instances.get_mut(source) {
                Some(instance) => {
                    instance.session_generation = session_generation;
                    adopted.push(GuestAdoption::Adopted(instance.clone()));
                }
                None => adopted.push(GuestAdoption::Missing),
            }
        }
        Ok(adopted)
    }

    /// Advance one target-local realization to ready (the target-local effect
    /// code calls this when its effect is serving).
    pub fn mark_ready(&self, source: &ResourceKey) -> Option<TargetResourceInstance> {
        let mut state = self.inner.lock();
        let instance = state.instances.get_mut(source)?;
        instance.state = TargetInstanceState::Ready;
        Some(instance.clone())
    }
}

/// One ComponentSession generation's capability over a [`GuestTargetRuntime`].
///
/// The generation is captured when the capability is minted and validated on
/// every operation, so a capability retained across a reconnect stops working
/// instead of acting for the new session.
#[derive(Debug)]
pub struct SessionBoundGuestTargetControl {
    runtime: Arc<GuestTargetRuntime>,
    session_generation: u64,
}

impl SessionBoundGuestTargetControl {
    /// Mint a capability for one authenticated session generation.
    pub fn new(
        runtime: Arc<GuestTargetRuntime>,
        session_generation: u64,
    ) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
        runtime.require_generation(session_generation)?;
        Ok(Arc::new(Self { runtime, session_generation }))
    }

    /// The session generation this capability is bound to.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// One capability only ever answers requests for its own generation; a
    /// request naming another one is refused before any effect.
    fn require_assignment(&self, assignment: &TargetControlAssignment) -> Result<(), GuestTargetError> {
        if assignment.session_generation() == self.session_generation {
            Ok(())
        } else {
            Err(GuestTargetError::StaleSessionGeneration)
        }
    }
}

#[async_trait]
impl GuestTargetControl for SessionBoundGuestTargetControl {
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError> {
        self.require_assignment(request.assignment())?;
        self.runtime.realize(request)
    }

    async fn observe(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<TargetObservation, GuestTargetError> {
        self.require_assignment(assignment)?;
        match self
            .runtime
            .observe(assignment.session_generation(), assignment.source())?
        {
            None => Ok(TargetObservation::Absent),
            Some(instance) => Ok(match instance.state() {
                TargetInstanceState::Realizing => {
                    TargetObservation::Realizing { session_generation: instance.session_generation() }
                }
                TargetInstanceState::Ready => {
                    TargetObservation::Ready { session_generation: instance.session_generation() }
                }
            }),
        }
    }

    async fn delete(&self, assignment: &TargetControlAssignment) -> Result<(), GuestTargetError> {
        self.require_assignment(assignment)?;
        self.runtime
            .delete(assignment.session_generation(), assignment.source())
            .map(|_| ())
    }

    async fn adopt(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<GuestAdoption, GuestTargetError> {
        self.require_assignment(assignment)?;
        let mut adopted = self
            .runtime
            .adopt(assignment.session_generation(), &[assignment.source().clone()])?;
        Ok(adopted.pop().unwrap_or(GuestAdoption::Missing))
    }
}


// ---------------------------------------------------------------------------
// Host-side target-control client (spec section 23.3)
// ---------------------------------------------------------------------------

impl TargetControlFrame {
    /// Encode this frame for the target-control channel.
    ///
    /// The payload is one JSON object whose explicit `protocol` member carries
    /// [`TARGET_CONTROL_PROTOCOL`]; the guest refuses anything else before it
    /// dispatches, and the reply carries the same member so a mismatched peer
    /// is detected on the host side too.
    pub fn encode(&self) -> Vec<u8> {
        let request = match &self.request {
            TargetControlRequest::Realize(request) => {
                let mut value = json_request("realize", request.assignment());
                value["spec"] = Value::String(hex_encode(request.spec()));
                value["specDigest"] = Value::String(request.spec_digest().to_owned());
                value["localHandle"] = Value::String(request.local_handle().to_owned());
                value
            }
            TargetControlRequest::Observe { assignment } => json_request("observe", assignment),
            TargetControlRequest::Delete { assignment } => json_request("delete", assignment),
            TargetControlRequest::Adopt { assignment } => json_request("adopt", assignment),
        };
        let frame = json_object(&[
            ("protocol", Value::String(self.protocol.clone())),
            ("request", request),
        ]);
        serde_json::to_vec(&frame).expect("target-control frames are JSON objects")
    }

    /// Decode one frame produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GuestTargetError> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| GuestTargetError::ProtocolMismatch)?;
        let protocol = value
            .get("protocol")
            .and_then(Value::as_str)
            .ok_or(GuestTargetError::ProtocolMismatch)?;
        let request = json_decode_request(
            value.get("request").ok_or(GuestTargetError::ProtocolMismatch)?,
        )?;
        Ok(Self { protocol: protocol.to_owned(), request })
    }
}

impl TargetControlResponse {
    /// Encode this response for the target-control channel.
    pub fn encode(&self) -> Vec<u8> {
        let response = match self {
            Self::Realized { realization } => json_object(&[
                ("kind", Value::String("realized".to_owned())),
                ("realization", json_realization(realization)),
            ]),
            Self::Observed(observation) => json_object(&[
                ("kind", Value::String("observed".to_owned())),
                ("observation", json_observation(*observation)),
            ]),
            Self::Deleted => json_object(&[("kind", Value::String("deleted".to_owned()))]),
            Self::Adopted(adoption) => json_object(&[
                ("kind", Value::String("adopted".to_owned())),
                ("adoption", json_adoption(adoption)),
            ]),
            Self::SessionUnavailable => {
                json_object(&[("kind", Value::String("session-unavailable".to_owned()))])
            }
        };
        let frame = json_object(&[("response", response)]);
        serde_json::to_vec(&frame).expect("target-control responses are JSON objects")
    }

    /// Decode one response produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GuestTargetError> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| GuestTargetError::ProtocolMismatch)?;
        let response = value
            .get("response")
            .ok_or(GuestTargetError::ProtocolMismatch)?;
        let kind = response
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(GuestTargetError::ProtocolMismatch)?;
        match kind {
            "realized" => Ok(Self::Realized {
                realization: json_decode_realization(
                    response.get("realization").ok_or(GuestTargetError::ProtocolMismatch)?,
                )?,
            }),
            "observed" => Ok(Self::Observed(json_decode_observation(
                response.get("observation").ok_or(GuestTargetError::ProtocolMismatch)?,
            )?)),
            "deleted" => Ok(Self::Deleted),
            "adopted" => Ok(Self::Adopted(json_decode_adoption(
                response.get("adoption").ok_or(GuestTargetError::ProtocolMismatch)?,
            )?)),
            "session-unavailable" => Ok(Self::SessionUnavailable),
            _ => Err(GuestTargetError::ProtocolMismatch),
        }
    }
}

/// The host-side target-control channel: one framed round trip.
///
/// The daemon's session-backed implementation sends the frame over the
/// authenticated ComponentSession; the in-process peer used by tests answers
/// it with [`GuestTargetRuntime::handle`]. Both speak the same frame.
#[async_trait]
pub trait TargetControlChannel: Send + Sync + fmt::Debug + 'static {
    /// Send one encoded request frame and return the encoded response frame.
    /// `SessionUnavailable` when no live session can carry it.
    async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError>;
}

/// Host-side target-control client: framing plus the session-generation fence.
///
/// The client is bound to one session generation. Every request must name that
/// generation; a request naming another one is refused locally, and a peer that
/// answers [`TargetControlResponse::SessionUnavailable`] is passed through
/// unchanged. Nothing here inherits authority across a reconnect.
#[derive(Debug, Clone)]
pub struct TargetControlClient<C: TargetControlChannel> {
    channel: C,
    session_generation: u64,
}

impl<C: TargetControlChannel> TargetControlClient<C> {
    /// Bind a channel to one authenticated session generation.
    pub fn new(channel: C, session_generation: u64) -> Result<Self, GuestTargetError> {
        if session_generation == 0 {
            return Err(GuestTargetError::SessionUnavailable);
        }
        Ok(Self { channel, session_generation })
    }

    /// The session generation this client is bound to.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// One fenced round trip: the request's own generation must be this
    /// client's, and the reply must be a frame this host speaks.
    pub async fn round_trip(
        &self,
        request: TargetControlRequest,
    ) -> Result<TargetControlResponse, GuestTargetError> {
        if request.session_generation() != self.session_generation {
            return Err(GuestTargetError::StaleSessionGeneration);
        }
        let reply = self.channel.call(TargetControlFrame::new(request).encode()).await?;
        TargetControlResponse::decode(&reply)
    }
}

#[async_trait]
impl<C: TargetControlChannel> GuestTargetControl for TargetControlClient<C> {
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError> {
        match self
            .round_trip(TargetControlRequest::Realize(request))
            .await?
        {
            TargetControlResponse::Realized { realization } => Ok(realization),
            TargetControlResponse::SessionUnavailable => Err(GuestTargetError::SessionUnavailable),
            _ => Err(GuestTargetError::ProtocolMismatch),
        }
    }

    async fn observe(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<TargetObservation, GuestTargetError> {
        match self
            .round_trip(TargetControlRequest::Observe { assignment: assignment.clone() })
            .await?
        {
            TargetControlResponse::Observed(observation) => Ok(observation),
            TargetControlResponse::SessionUnavailable => Err(GuestTargetError::SessionUnavailable),
            _ => Err(GuestTargetError::ProtocolMismatch),
        }
    }

    async fn delete(&self, assignment: &TargetControlAssignment) -> Result<(), GuestTargetError> {
        match self
            .round_trip(TargetControlRequest::Delete { assignment: assignment.clone() })
            .await?
        {
            TargetControlResponse::Deleted => Ok(()),
            TargetControlResponse::SessionUnavailable => Err(GuestTargetError::SessionUnavailable),
            _ => Err(GuestTargetError::ProtocolMismatch),
        }
    }

    async fn adopt(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<GuestAdoption, GuestTargetError> {
        match self
            .round_trip(TargetControlRequest::Adopt { assignment: assignment.clone() })
            .await?
        {
            TargetControlResponse::Adopted(adoption) => Ok(adoption),
            TargetControlResponse::SessionUnavailable => Err(GuestTargetError::SessionUnavailable),
            _ => Err(GuestTargetError::ProtocolMismatch),
        }
    }
}

fn json_object(members: &[(&str, Value)]) -> Value {
    let mut object = serde_json::Map::with_capacity(members.len());
    for (name, value) in members {
        object.insert((*name).to_owned(), value.clone());
    }
    Value::Object(object)
}

fn json_key(key: &ResourceKey) -> Value {
    json_object(&[
        ("zone", Value::String(key.zone.clone())),
        ("typeName", Value::String(key.type_name.clone())),
        ("name", Value::String(key.name.clone())),
    ])
}

fn json_assignment(assignment: &TargetControlAssignment) -> Value {
    json_object(&[
        ("source", json_key(assignment.source())),
        ("sourceUid", Value::String(hex_encode(&assignment.source_uid()[..]))),
        (
            "assignmentGeneration",
            Value::from(assignment.assignment_generation()),
        ),
        (
            "sessionGeneration",
            Value::from(assignment.session_generation()),
        ),
    ])
}

fn json_request(kind: &str, assignment: &TargetControlAssignment) -> Value {
    json_object(&[
        ("kind", Value::String(kind.to_owned())),
        ("assignment", json_assignment(assignment)),
    ])
}

fn json_realization(instance: &TargetResourceInstance) -> Value {
    json_object(&[
        ("source", json_key(instance.source())),
        ("sourceUid", Value::String(hex_encode(&instance.source_uid()[..]))),
        (
            "assignmentGeneration",
            Value::from(instance.assignment_generation()),
        ),
        (
            "sessionGeneration",
            Value::from(instance.session_generation()),
        ),
        ("localHandle", Value::String(instance.local_handle().to_owned())),
        ("specDigest", Value::String(instance.spec_digest().to_owned())),
        ("state", Value::String(instance.state().as_str().to_owned())),
    ])
}

fn json_observation(observation: TargetObservation) -> Value {
    match observation {
        TargetObservation::Absent => json_object(&[("kind", Value::String("absent".to_owned()))]),
        TargetObservation::Realizing { session_generation } => json_object(&[
            ("kind", Value::String("realizing".to_owned())),
            ("sessionGeneration", Value::from(session_generation)),
        ]),
        TargetObservation::Ready { session_generation } => json_object(&[
            ("kind", Value::String("ready".to_owned())),
            ("sessionGeneration", Value::from(session_generation)),
        ]),
        TargetObservation::Unavailable => {
            json_object(&[("kind", Value::String("unavailable".to_owned()))])
        }
    }
}

fn json_adoption(adoption: &GuestAdoption) -> Value {
    match adoption {
        GuestAdoption::Adopted(instance) => json_object(&[
            ("kind", Value::String("adopted".to_owned())),
            ("realization", json_realization(instance)),
        ]),
        GuestAdoption::Missing => json_object(&[("kind", Value::String("missing".to_owned()))]),
    }
}

fn json_decode_key(value: &Value) -> Result<ResourceKey, GuestTargetError> {
    let zone = value
        .get("zone")
        .and_then(Value::as_str)
        .ok_or(GuestTargetError::ProtocolMismatch)?;
    let type_name = value
        .get("typeName")
        .and_then(Value::as_str)
        .ok_or(GuestTargetError::ProtocolMismatch)?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or(GuestTargetError::ProtocolMismatch)?;
    Ok(ResourceKey::new(zone, type_name, name))
}

fn json_decode_assignment(value: &Value) -> Result<TargetControlAssignment, GuestTargetError> {
    Ok(TargetControlAssignment::new(
        json_decode_key(value.get("source").ok_or(GuestTargetError::ProtocolMismatch)?)?,
        hex_decode(value.get("sourceUid").and_then(Value::as_str))?,
        value
            .get("assignmentGeneration")
            .and_then(Value::as_u64)
            .ok_or(GuestTargetError::ProtocolMismatch)?,
        value
            .get("sessionGeneration")
            .and_then(Value::as_u64)
            .ok_or(GuestTargetError::ProtocolMismatch)?,
    ))
}

fn json_decode_request(value: &Value) -> Result<TargetControlRequest, GuestTargetError> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(GuestTargetError::ProtocolMismatch)?;
    let assignment_value = value.get("assignment").ok_or(GuestTargetError::ProtocolMismatch)?;
    let assignment = json_decode_assignment(assignment_value)?;
    match kind {
        "realize" => Ok(TargetControlRequest::Realize(GuestRealizeRequest::new(
            assignment,
            hex_decode_bytes(value.get("spec").and_then(Value::as_str))?,
            value
                .get("specDigest")
                .and_then(Value::as_str)
                .ok_or(GuestTargetError::ProtocolMismatch)?
                .to_owned(),
            value
                .get("localHandle")
                .and_then(Value::as_str)
                .ok_or(GuestTargetError::ProtocolMismatch)?
                .to_owned(),
        ))),
        "observe" => Ok(TargetControlRequest::Observe { assignment }),
        "delete" => Ok(TargetControlRequest::Delete { assignment }),
        "adopt" => Ok(TargetControlRequest::Adopt { assignment }),
        _ => Err(GuestTargetError::ProtocolMismatch),
    }
}

fn json_decode_realization(value: &Value) -> Result<TargetResourceInstance, GuestTargetError> {
    let state = match value.get("state").and_then(Value::as_str) {
        Some("realizing") => TargetInstanceState::Realizing,
        Some("ready") => TargetInstanceState::Ready,
        _ => return Err(GuestTargetError::ProtocolMismatch),
    };
    Ok(TargetResourceInstance::new(
        json_decode_key(value.get("source").ok_or(GuestTargetError::ProtocolMismatch)?)?,
        hex_decode(value.get("sourceUid").and_then(Value::as_str))?,
        value
            .get("assignmentGeneration")
            .and_then(Value::as_u64)
            .ok_or(GuestTargetError::ProtocolMismatch)?,
        value
            .get("sessionGeneration")
            .and_then(Value::as_u64)
            .ok_or(GuestTargetError::ProtocolMismatch)?,
        value
            .get("localHandle")
            .and_then(Value::as_str)
            .ok_or(GuestTargetError::ProtocolMismatch)?
            .to_owned(),
        value
            .get("specDigest")
            .and_then(Value::as_str)
            .ok_or(GuestTargetError::ProtocolMismatch)?
            .to_owned(),
        state,
    ))
}

fn json_decode_observation(value: &Value) -> Result<TargetObservation, GuestTargetError> {
    match value.get("kind").and_then(Value::as_str) {
        Some("absent") => Ok(TargetObservation::Absent),
        Some("realizing") => Ok(TargetObservation::Realizing {
            session_generation: value
                .get("sessionGeneration")
                .and_then(Value::as_u64)
                .ok_or(GuestTargetError::ProtocolMismatch)?,
        }),
        Some("ready") => Ok(TargetObservation::Ready {
            session_generation: value
                .get("sessionGeneration")
                .and_then(Value::as_u64)
                .ok_or(GuestTargetError::ProtocolMismatch)?,
        }),
        Some("unavailable") => Ok(TargetObservation::Unavailable),
        _ => Err(GuestTargetError::ProtocolMismatch),
    }
}

fn json_decode_adoption(value: &Value) -> Result<GuestAdoption, GuestTargetError> {
    match value.get("kind").and_then(Value::as_str) {
        Some("adopted") => Ok(GuestAdoption::Adopted(json_decode_realization(
            value.get("realization").ok_or(GuestTargetError::ProtocolMismatch)?,
        )?)),
        Some("missing") => Ok(GuestAdoption::Missing),
        _ => Err(GuestTargetError::ProtocolMismatch),
    }
}

/// Lowercase hex of a byte string (the frame carries no bare binary).
fn hex_encode(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

/// Decode one lowercase-hex byte string (the request's spec payload).
fn hex_decode_bytes(value: Option<&str>) -> Result<Vec<u8>, GuestTargetError> {
    let value = value.ok_or(GuestTargetError::ProtocolMismatch)?;
    if value.len() % 2 != 0 {
        return Err(GuestTargetError::ProtocolMismatch);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in 0..value.len() / 2 {
        bytes.push(
            u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
                .map_err(|_| GuestTargetError::ProtocolMismatch)?,
        );
    }
    Ok(bytes)
}

/// Decode one lowercase-hex identity field (16 bytes) back into bytes.
fn hex_decode(value: Option<&str>) -> Result<[u8; 16], GuestTargetError> {
    let value = value.ok_or(GuestTargetError::ProtocolMismatch)?;
    if value.len() != 32 {
        return Err(GuestTargetError::ProtocolMismatch);
    }
    let mut bytes = [0_u8; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| GuestTargetError::ProtocolMismatch)?;
    }
    Ok(bytes)
}

/// Closed Guest target-control failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTargetError {
    /// No authenticated session generation is bound.
    SessionUnavailable,
    /// The frame carried a protocol token this daemon does not answer.
    ProtocolMismatch,
    /// The request belongs to a generation that is not the live one.
    StaleSessionGeneration,
    /// A reconnect carried a generation older than the live one.
    SessionGenerationRegression,
}

impl fmt::Display for GuestTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SessionUnavailable => "guest-target-session-unavailable",
            Self::ProtocolMismatch => "guest-target-protocol-mismatch",
            Self::StaleSessionGeneration => "guest-target-stale-session-generation",
            Self::SessionGenerationRegression => "guest-target-session-generation-regression",
        })
    }
}

impl std::error::Error for GuestTargetError {}

/// Stable ordering for resource identities inside a target.
fn identity_order(key: &ResourceKey) -> (&str, &str, &str) {
    (&key.zone, &key.type_name, &key.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest() -> TargetRef {
        TargetRef::guest("work-vm").expect("guest ref")
    }

    fn source(name: &str) -> ResourceKey {
        ResourceKey::new("work", "Process", name)
    }

    /// The test source's assignment for one session generation.
    fn assignment(session_generation: u64) -> TargetControlAssignment {
        TargetControlAssignment::new(source("foo"), [7; 16], 1, session_generation)
    }

    /// One realize request bound to a test session generation.
    fn realize_request(
        source: ResourceKey,
        assignment_generation: u64,
        session_generation: u64,
        local_handle: &str,
    ) -> GuestRealizeRequest {
        GuestRealizeRequest::new(
            TargetControlAssignment::new(
                source,
                [7; 16],
                assignment_generation,
                session_generation,
            ),
            br#"{"providerRef":"Provider/test"}"#.to_vec(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000002",
            local_handle,
        )
    }

    #[tokio::test]
    async fn realization_is_idempotent_per_source_and_never_a_second_resource() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let control = runtime.control(1).expect("control");

        let first = control
            .realize(realize_request(source("foo"), 3, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");
        let second = control
            .realize(realize_request(source("foo"), 4, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize again");

        assert_eq!(runtime.instances().len(), 1, "one realization per host-owned resource");
        assert_eq!(first.source(), second.source());
        assert_eq!(second.assignment_generation(), 4, "the same instance tracks the new desired generation");
        assert_eq!(second.source().zone, "work", "the guest keeps the authority-zone identity");
    }

    #[tokio::test]
    async fn a_session_bound_capability_stops_working_after_reconnect() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let old = runtime.control(1).expect("control");
        old.realize(realize_request(source("foo"), 1, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");

        runtime.bind_session(2).expect("reconnect generation");
        assert_eq!(
            old.observe(&assignment(1)).await.err(),
            Some(GuestTargetError::StaleSessionGeneration),
            "a capability retained across a reconnect cannot act for the new session"
        );
        let current = runtime.control(2).expect("control");
        assert!(matches!(
            current.observe(&assignment(2)).await.expect("observe"),
            TargetObservation::Realizing { .. }
        ));
    }

    #[tokio::test]
    async fn adoption_rebinds_present_realizations_and_reports_missing_sources() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        runtime
            .control(1)
            .expect("control")
            .realize(realize_request(source("foo"), 1, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");
        runtime.bind_session(2).expect("reconnect generation");

        let control = runtime.control(2).expect("control");
        match control.adopt(&assignment(2)).await.expect("adopt") {
            GuestAdoption::Adopted(instance) => {
                assert_eq!(instance.session_generation(), 2);
                assert_eq!(instance.local_handle(), "/run/d2b/foo.sock");
            }
            GuestAdoption::Missing => panic!("a present realization is adopted, not recreated"),
        }
        assert_eq!(control.adopt(&assignment(2)).await.expect("adopt"), GuestAdoption::Adopted(
            runtime.instance(&source("foo")).expect("instance")
        ));
        assert_eq!(runtime.instances().len(), 1);
    }


    #[test]
    fn a_request_naming_a_stale_generation_answers_session_unavailable_and_does_nothing() {
        let runtime = GuestTargetRuntime::new(guest());
        runtime.bind_session(2).expect("bind session");
        let source = source("foo");
        let assignment = TargetControlAssignment::new(source.clone(), [7; 16], 1, 1);

        // A realize from the lost session: no instance appears.
        let realize = TargetControlRequest::Realize(realize_request(source.clone(), 1, 1, "/run/d2b/foo.sock"));
        assert_eq!(runtime.handle(realize), TargetControlResponse::SessionUnavailable);
        assert!(runtime.instance(&source).is_none(), "a stale realize performs no effect");

        // Observe/Delete/Adopt from the lost session answer the same way.
        assert_eq!(
            runtime.handle(TargetControlRequest::Observe { assignment: assignment.clone() }),
            TargetControlResponse::SessionUnavailable
        );
        assert_eq!(
            runtime.handle(TargetControlRequest::Delete { assignment: assignment.clone() }),
            TargetControlResponse::SessionUnavailable
        );
        assert_eq!(
            runtime.handle(TargetControlRequest::Adopt { assignment: assignment.clone() }),
            TargetControlResponse::SessionUnavailable
        );

        // The live generation works and its answers are typed.
        let live = TargetControlAssignment::new(source.clone(), [7; 16], 1, 2);
        assert!(matches!(
            runtime.handle(TargetControlRequest::Realize(realize_request(source.clone(), 1, 2, "/run/d2b/foo.sock"))),
            TargetControlResponse::Realized { .. }
        ));
        assert_eq!(
            runtime.handle(TargetControlRequest::Observe { assignment: live.clone() }),
            TargetControlResponse::Observed(TargetObservation::Realizing { session_generation: 2 })
        );
        assert_eq!(
            runtime.handle(TargetControlRequest::Adopt { assignment: live.clone() }),
            TargetControlResponse::Adopted(GuestAdoption::Adopted(
                runtime.instance(&source).expect("instance")
            ))
        );
        assert_eq!(
            runtime.handle(TargetControlRequest::Delete { assignment: live }),
            TargetControlResponse::Deleted
        );
        assert!(runtime.instance(&source).is_none());
    }

    #[test]
    fn frames_carry_one_protocol_token_and_refuse_any_other() {
        let request = TargetControlRequest::Observe {
            assignment: TargetControlAssignment::new(source("foo"), [7; 16], 1, 1),
        };
        let frame = TargetControlFrame::new(request.clone());
        assert_eq!(frame.protocol(), TARGET_CONTROL_PROTOCOL);
        assert_eq!(frame.clone().into_request().expect("current protocol"), request);

        let foreign = TargetControlFrame { protocol: "d2b.target-control.v0".to_owned(), request };
        assert_eq!(foreign.into_request().err(), Some(GuestTargetError::ProtocolMismatch));
    }

    #[test]
    fn the_spec_commitment_covers_the_domain_and_the_exact_bytes() {
        let spec = br#"{"providerRef":"Provider/test"}"#;
        let digest = target_local_spec_digest(spec);
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), 7 + 64);
        assert_eq!(digest, target_local_spec_digest(spec));
        assert_ne!(digest, target_local_spec_digest(br#"{"providerRef":"Provider/other"}"#));
        assert_ne!(digest, target_local_spec_digest(b""));
    }

    /// In-process target-control peer: the same frames the daemon sends,
    /// answered by the published guest dispatch. This is the host half's fake
    /// peer, and it proves the codec is symmetric with the guest's decode.
    #[derive(Debug)]
    struct InProcessPeer {
        runtime: Arc<GuestTargetRuntime>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl InProcessPeer {
        fn new(runtime: Arc<GuestTargetRuntime>) -> Self {
            Self { runtime, calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)) }
        }

    }

    #[async_trait]
    impl TargetControlChannel for InProcessPeer {
        async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let request = TargetControlFrame::decode(&frame)?.into_request()?;
            Ok(self.runtime.handle(request).encode())
        }
    }

    #[tokio::test]
    async fn the_host_client_round_trips_the_exact_frame_the_guest_dispatches() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let peer = InProcessPeer::new(Arc::clone(&runtime));
        let client = TargetControlClient::new(peer, 1).expect("client");

        let request = realize_request(source("foo"), 4, 1, "/run/d2b/foo.sock");
        let realization = client.realize(request.clone()).await.expect("realize");
        assert_eq!(realization.source(), &source("foo"));
        assert_eq!(realization.assignment_generation(), 4);
        assert_eq!(realization.session_generation(), 1);
        assert_eq!(realization.local_handle(), "/run/d2b/foo.sock");
        assert_eq!(
            realization.spec_digest(),
            request.spec_digest(),
            "the spec commitment survives the frame in both directions"
        );

        assert_eq!(
            client
                .observe(&TargetControlAssignment::new(source("foo"), [7; 16], 4, 1))
                .await
                .expect("observe"),
            TargetObservation::Realizing { session_generation: 1 }
        );
        assert_eq!(
            client
                .adopt(&TargetControlAssignment::new(source("foo"), [7; 16], 4, 1))
                .await
                .expect("adopt"),
            GuestAdoption::Adopted(realization)
        );
        assert_eq!(
            client
                .delete(&TargetControlAssignment::new(source("foo"), [7; 16], 4, 1))
                .await,
            Ok(())
        );
        assert_eq!(
            client
                .observe(&TargetControlAssignment::new(source("foo"), [7; 16], 4, 1))
                .await
                .expect("observe"),
            TargetObservation::Absent
        );
    }

    #[tokio::test]
    async fn a_stale_generation_is_refused_locally_and_never_reaches_the_peer() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(2).expect("bind session");
        let peer = InProcessPeer::new(Arc::clone(&runtime));
        let client = TargetControlClient::new(peer, 2).expect("client");

        assert_eq!(
            client.realize(realize_request(source("foo"), 1, 1, "/run/d2b/foo.sock")).await.err(),
            Some(GuestTargetError::StaleSessionGeneration),
        );
        assert!(runtime.instance(&source("foo")).is_none(), "a refused request performs no effect");
        // No channel round trip happened at all: the fence is local.
        let client = TargetControlClient::new(InProcessPeer::new(Arc::clone(&runtime)), 2)
            .expect("client");
        assert_eq!(
            client
                .observe(&TargetControlAssignment::new(source("foo"), [7; 16], 1, 1))
                .await
                .err(),
            Some(GuestTargetError::StaleSessionGeneration)
        );
    }

    #[tokio::test]
    async fn a_lost_session_answers_session_unavailable_over_the_wire() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let peer = InProcessPeer::new(Arc::clone(&runtime));
        let calls = peer.calls.clone();
        let client = TargetControlClient::new(peer, 1).expect("client");
        client
            .realize(realize_request(source("foo"), 1, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");

        // The live generation advances; the client still names the lost one.
        runtime.bind_session(2).expect("reconnect");
        assert_eq!(
            client
                .observe(&TargetControlAssignment::new(source("foo"), [7; 16], 1, 1))
                .await
                .err(),
            Some(GuestTargetError::SessionUnavailable),
            "the peer's answer is passed through as SessionUnavailable"
        );
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "the request did reach the peer: this is a wire answer, not a local refusal"
        );
    }

    #[tokio::test]
    async fn adoption_over_the_wire_rebinds_instead_of_inheriting() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let first = TargetControlClient::new(InProcessPeer::new(Arc::clone(&runtime)), 1)
            .expect("client");
        let stored = first
            .realize(realize_request(source("foo"), 1, 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");

        runtime.bind_session(2).expect("reconnect");
        let second = TargetControlClient::new(InProcessPeer::new(Arc::clone(&runtime)), 2)
            .expect("client");
        match second
            .adopt(&TargetControlAssignment::new(source("foo"), [7; 16], 1, 2))
            .await
            .expect("adopt")
        {
            GuestAdoption::Adopted(instance) => {
                assert_eq!(instance.session_generation(), 2, "the realization is re-bound");
                assert_eq!(instance.local_handle(), stored.local_handle());
                assert_eq!(instance.spec_digest(), stored.spec_digest());
            }
            GuestAdoption::Missing => panic!("a present realization is adopted, not recreated"),
        }
        assert_eq!(runtime.instances().len(), 1, "adoption never mints a duplicate");
    }

    #[test]
    fn session_binding_refuses_a_generation_regression() {
        let runtime = GuestTargetRuntime::new(guest());
        runtime.bind_session(2).expect("bind session");
        assert_eq!(runtime.bind_session(2), Ok(()), "rebinding the live generation is a no-op");
        assert_eq!(
            runtime.bind_session(1),
            Err(GuestTargetError::SessionGenerationRegression)
        );
        assert_eq!(runtime.session_generation(), Some(2));
    }
}
