//! Host-side target-control channel over the authenticated Guest
//! ComponentSession (U13, spec section 23.3).
//!
//! One frame per call on the frozen target-control service. The channel does
//! not re-implement session bookkeeping: the frame names the session
//! generation, the guest refuses any other one, and a session that is gone
//! answers `SessionUnavailable`.
//!
//! The session itself is the daemon's: the family declares the small port the
//! channel needs ([`GuestTargetSession`] - a liveness answer and one
//! request/answer round trip) and the daemon implements it over its
//! authenticated `GuestComponentSession` client, so this crate carries no
//! session runtime and no carrier of its own.
//!
//! ## The common graph contract
//!
//! [`GuestTargetContract`] is the part every Guest implementation shares: the
//! graph evidence one accepted parent session presents
//! ([`GuestParentSessionEvidence`]), the per-source ownership that evidence
//! admits ([`GuestTargetBinding`]), and the rules that fence a target-local
//! effect on them. The declared Provider is carried as graph data and is never
//! matched, so a local VM provider, a media-backed provider, and a remote cloud
//! provider consume the same contract and none of them waits for another.
//!
//! The contract is the *parent-side* view: it answers whether one assignment
//! may act, and it is what a channel binds before it sends. The Guest-side
//! consumption lives in [`crate::target_service`].

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_resource_runtime::guest_target::{
    GuestTargetControl, GuestTargetError, TargetControlAssignment, TargetControlChannel,
    TargetControlClient, TargetControlFrame, TargetControlRequest, TARGET_CONTROL_METHOD,
    TARGET_CONTROL_SERVICE,
};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::target::TargetRef;
use d2bd_runtime::target_runtime::GuestParentSessionEvidence;

use crate::target_service::GuestTargetRefusal;

/// Bounded deadline for one target-control round trip.
const TARGET_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

/// One authenticated Guest session a target-control channel rides.
///
/// The daemon implements this over the accepted session's own carrier: the
/// liveness answer is the session's live generation, and one request is one
/// round trip on the session's authenticated client. A session that is gone
/// answers [`GuestTargetError::SessionUnavailable`], never a retry on another
/// generation.
#[async_trait]
pub trait GuestTargetSession: Send + Sync + 'static {
    /// Whether this session is still live.
    fn is_live(&self) -> bool;

    /// Carry one target-control request over the session.
    async fn request(
        &self,
        request: ttrpc::Request,
    ) -> Result<ttrpc::Response, GuestTargetError>;
}

/// One target-control channel over one authenticated Guest session.
pub struct SessionTargetControlChannel<S: GuestTargetSession> {
    session: S,
}

/// The channel names the session it rides; the session's own shape is the
/// daemon's and is not rendered here.
impl<S: GuestTargetSession> std::fmt::Debug for SessionTargetControlChannel<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionTargetControlChannel")
            .finish_non_exhaustive()
    }
}

impl<S: GuestTargetSession> SessionTargetControlChannel<S> {
    /// Wrap one authenticated Guest session.
    pub fn new(session: S) -> Self {
        Self { session }
    }
}

#[async_trait]
impl<S: GuestTargetSession> TargetControlChannel for SessionTargetControlChannel<S> {
    async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError> {
        // A session that is no longer live cannot carry a target-control
        // request: the answer is the closed `SessionUnavailable`, never a
        // retry on another generation (R21, R28).
        if !self.session.is_live() {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let request = ttrpc::Request {
            service: TARGET_CONTROL_SERVICE.to_owned(),
            method: TARGET_CONTROL_METHOD.to_owned(),
            payload: frame,
            timeout_nano: TARGET_CONTROL_TIMEOUT.as_nanos() as i64,
            ..ttrpc::Request::default()
        };
        let response = self.session.request(request).await?;
        Ok(response.payload)
    }
}

/// The target-control handle of one live Guest session generation.
///
/// The returned handle is generation-bound: a request naming another
/// generation is refused host-side, and the guest refuses it again on the
/// wire.
///
/// # Errors
///
/// Returns the [`GuestTargetError`] the client construction reports when
/// the session generation binding cannot be established.
pub fn session_target_control<S: GuestTargetSession>(
    session: S,
    session_generation: u64,
) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
    let client =
        TargetControlClient::new(SessionTargetControlChannel::new(session), session_generation)?;
    Ok(Arc::new(client))
}

/// The directory target reference of an authenticated Guest session.
pub fn guest_target_ref(guest: &ResourceRef) -> Option<TargetRef> {
    if guest.resource_type().as_str() != "Guest" {
        return None;
    }
    TargetRef::guest(guest.name().as_str()).ok()
}

// ---------------------------------------------------------------------------
// The common Guest target/session contract
// ---------------------------------------------------------------------------

/// The ownership one admitted assignment leaves behind on this Guest.
///
/// A binding is the contract's memory of a Host-zone source: the exact source
/// key, the exact source uid the accepted graph bound to it, the desired
/// generation the assignment realizes, and the session generation that last
/// realized or adopted it. It is deliberately not a desired spec and never
/// enters the Guest's resource namespace.
///
/// A binding survives a lost session unchanged. That is the conservative
/// reading of ownership: the Guest still owns the same source for the same
/// uid, and a reconnecting session must re-adopt it rather than mint a second
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestTargetBinding {
    source: ResourceKey,
    source_uid: [u8; 16],
    assignment_generation: u64,
    session_generation: u64,
}

impl GuestTargetBinding {
    /// Borrow the owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// Return the owning resource's stable uid.
    pub fn source_uid(&self) -> [u8; 16] {
        self.source_uid
    }

    /// Return the desired generation this binding realizes.
    pub const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    /// Return the session generation that last realized or adopted this
    /// binding.
    ///
    /// A lost session does not advance it: only an explicit adoption rebinds
    /// a realization to a live session.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }
}

/// The common Guest target/session contract every Guest implementation shares.
///
/// The contract holds the graph evidence of the accepted parent session and
/// the per-source ownership that evidence admits, and it answers one question
/// for every target-control request: may this assignment act for this Guest
/// right now?
///
/// The rules are the same for every declared Guest Provider. Nothing here
/// reads which Provider the Guest names, so a second implementation consumes
/// this contract without waiting for the first to exist.
///
/// The fence, evaluated in order, and all of it before any state exists:
///
/// - no live session, or a request naming another generation, is
///   [`GuestTargetRefusal::SessionUnavailable`];
/// - a source outside the evidence's authority Zone is
///   [`GuestTargetRefusal::ForeignZone`];
/// - a target reference other than this Guest's is
///   [`GuestTargetRefusal::TargetMismatch`];
/// - a source whose uid is not the uid this Guest already owns is
///   [`GuestTargetRefusal::SourceReplaced`] - a replaced source can never
///   inherit the previous source's realization;
/// - a desired generation older than the one already admitted is
///   [`GuestTargetRefusal::AssignmentRegression`].
///
/// A newer desired generation for the same source and uid is the ordinary
/// update path, and a repeated generation is the ordinary idempotent path.
#[derive(Debug)]
pub struct GuestTargetContract {
    evidence: GuestParentSessionEvidence,
    target: TargetRef,
    live_session: Option<u64>,
    /// The newest generation this contract ever connected. It survives a lost
    /// session, so a reconnect can never reuse a generation it already gave
    /// up.
    last_generation: u64,
    bindings: HashMap<ResourceKey, GuestTargetBinding>,
}

impl GuestTargetContract {
    /// Bind the contract to one accepted parent session's graph evidence.
    ///
    /// No session is live yet: [`Self::connect`] admits the first one.
    ///
    /// # Errors
    ///
    /// Returns `None` when the evidence's enrolled execution reference is not
    /// a canonical `Guest/<name>`, so a Guest never builds a contract for a
    /// target it cannot name.
    pub fn bind(evidence: GuestParentSessionEvidence) -> Option<Self> {
        let target = guest_target_ref(evidence.guest_ref())?;
        Some(Self {
            evidence,
            target,
            live_session: None,
            last_generation: 0,
            bindings: HashMap::new(),
        })
    }

    /// Borrow the graph evidence this contract is fenced on.
    pub const fn evidence(&self) -> &GuestParentSessionEvidence {
        &self.evidence
    }

    /// The directory target this contract realizes for.
    pub const fn target(&self) -> &TargetRef {
        &self.target
    }

    /// The live authenticated session generation, when one is connected.
    pub const fn session_generation(&self) -> Option<u64> {
        self.live_session
    }

    /// Connect the contract to the live parent session generation.
    ///
    /// Reconnecting retains every recorded binding: a lost session gives up
    /// its live generation, never its ownership.
    ///
    /// # Errors
    ///
    /// Returns [`GuestTargetRefusal::SessionUnavailable`] for generation zero
    /// and [`GuestTargetRefusal::StaleSessionGeneration`] for a generation
    /// that is not strictly newer than the last one this contract connected -
    /// a reconnect can never inherit, or re-use, an older session's authority,
    /// even when that session is already gone.
    pub fn connect(&mut self, session_generation: u64) -> Result<(), GuestTargetRefusal> {
        if session_generation == 0 {
            return Err(GuestTargetRefusal::SessionUnavailable);
        }
        if session_generation <= self.last_generation {
            return Err(GuestTargetRefusal::StaleSessionGeneration);
        }
        self.live_session = Some(session_generation);
        self.last_generation = session_generation;
        Ok(())
    }

    /// Record the loss of the live parent session.
    ///
    /// Every binding is retained with the source key, source uid, and desired
    /// generation it was admitted with, and no binding is advanced to the new
    /// session. Nothing can be admitted while no session is live, so a lost
    /// session cannot mint fresh Host authority; it can only hand its
    /// ownership to a reconnecting session that re-adopts it.
    ///
    /// # Errors
    ///
    /// Returns [`GuestTargetRefusal::StaleSessionGeneration`] when
    /// `session_generation` is not the live one, so a retained caller cannot
    /// close a session it does not own.
    pub fn disconnect(&mut self, session_generation: u64) -> Result<(), GuestTargetRefusal> {
        if self.live_session != Some(session_generation) {
            return Err(GuestTargetRefusal::StaleSessionGeneration);
        }
        self.live_session = None;
        Ok(())
    }

    /// Check one assignment against the fence without recording anything.
    ///
    /// This is the read path - an observation, or a delete of a source whose
    /// realization may already be gone. Neither mints ownership: a Guest that
    /// merely looked at a source does not come to own it.
    ///
    /// # Errors
    ///
    /// Returns the [`GuestTargetRefusal`] the assignment fails on.
    pub fn check(
        &self,
        target: &TargetRef,
        assignment: &TargetControlAssignment,
    ) -> Result<(), GuestTargetRefusal> {
        let Some(live) = self.live_session else {
            return Err(GuestTargetRefusal::SessionUnavailable);
        };
        if target != &self.target {
            return Err(GuestTargetRefusal::TargetMismatch);
        }
        if assignment.session_generation() != live {
            return Err(GuestTargetRefusal::StaleSessionGeneration);
        }
        if assignment.source().zone != self.evidence.zone().as_str() {
            return Err(GuestTargetRefusal::ForeignZone);
        }
        match self.bindings.get(assignment.source()) {
            Some(recorded) if recorded.source_uid != *assignment.source_uid() => {
                Err(GuestTargetRefusal::SourceReplaced)
            }
            Some(recorded) if assignment.assignment_generation() < recorded.assignment_generation => {
                Err(GuestTargetRefusal::AssignmentRegression)
            }
            _ => Ok(()),
        }
    }

    /// Admit one assignment, returning the ownership it holds afterwards.
    ///
    /// # Errors
    ///
    /// Returns the [`GuestTargetRefusal`] [`Self::check`] reports; the
    /// contract's recorded ownership is unchanged in every one of those cases.
    pub fn admit(
        &mut self,
        target: &TargetRef,
        assignment: &TargetControlAssignment,
    ) -> Result<GuestTargetBinding, GuestTargetRefusal> {
        self.check(target, assignment)?;
        let live = self
            .live_session
            .ok_or(GuestTargetRefusal::SessionUnavailable)?;
        let binding = GuestTargetBinding {
            source: assignment.source().clone(),
            source_uid: *assignment.source_uid(),
            assignment_generation: assignment.assignment_generation(),
            session_generation: live,
        };
        self.bindings.insert(assignment.source().clone(), binding.clone());
        Ok(binding)
    }

    /// The ownership this contract holds for one source, when it holds any.
    pub fn binding(&self, source: &ResourceKey) -> Option<&GuestTargetBinding> {
        self.bindings.get(source)
    }

    /// Every binding this contract still owns, in source-key order.
    ///
    /// Bindings are retained across a lost session, so this list is the
    /// conservative ownership a reconnecting session must re-adopt.
    pub fn bindings(&self) -> Vec<GuestTargetBinding> {
        let mut bindings = self.bindings.values().cloned().collect::<Vec<_>>();
        bindings.sort_by(|left, right| {
            (
                left.source().zone.as_str(),
                left.source().type_name.as_str(),
                left.source().name.as_str(),
            )
                .cmp(&(
                    right.source().zone.as_str(),
                    right.source().type_name.as_str(),
                    right.source().name.as_str(),
                ))
        });
        bindings
    }

    /// Release one source's ownership.
    ///
    /// This is the fenced delete: the Host side retires a source exactly once,
    /// and only then may a later assignment for the same key carry a different
    /// uid.
    pub fn release(&mut self, source: &ResourceKey) -> Option<GuestTargetBinding> {
        self.bindings.remove(source)
    }
}

/// The graph-backed target-control handle of one accepted parent session.
///
/// The handle is the contract's own channel: every frame it sends is admitted
/// by the same rules the Guest applies on arrival, so a stale generation or a
/// replaced source is refused here instead of consuming a round trip - and is
/// refused there again on the wire.
///
/// # Errors
///
/// Returns the [`GuestTargetError`] the client construction reports when the
/// session generation binding cannot be established.
pub fn graph_target_control<S: GuestTargetSession>(
    contract: Arc<std::sync::Mutex<GuestTargetContract>>,
    session: S,
    session_generation: u64,
) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
    let client = TargetControlClient::new(
        ContractFencedChannel { contract, session },
        session_generation,
    )?;
    Ok(Arc::new(client))
}

/// A session channel whose frames pass the common contract before they are
/// carried.
struct ContractFencedChannel<S: GuestTargetSession> {
    contract: Arc<Mutex<GuestTargetContract>>,
    session: S,
}

impl<S: GuestTargetSession> std::fmt::Debug for ContractFencedChannel<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContractFencedChannel")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: GuestTargetSession> TargetControlChannel for ContractFencedChannel<S> {
    async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError> {
        let request = TargetControlFrame::decode(&frame)?.into_request()?;
        self.admit(&request)?;
        if !self.session.is_live() {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let request = ttrpc::Request {
            service: TARGET_CONTROL_SERVICE.to_owned(),
            method: TARGET_CONTROL_METHOD.to_owned(),
            payload: frame,
            timeout_nano: TARGET_CONTROL_TIMEOUT.as_nanos() as i64,
            ..ttrpc::Request::default()
        };
        let response = self.session.request(request).await?;
        Ok(response.payload)
    }
}

impl<S: GuestTargetSession> ContractFencedChannel<S> {
    /// Admit one request against the contract, in its own critical section so
    /// no guard is held across the request round trip.
    fn admit(&self, request: &TargetControlRequest) -> Result<(), GuestTargetError> {
        let mut contract = self
            .contract
            .lock()
            .map_err(|_| GuestTargetError::SessionUnavailable)?;
        let target = contract.target().clone();
        match request {
            TargetControlRequest::Realize(_) | TargetControlRequest::Adopt { .. } => contract
                .admit(&target, request_assignment(request))
                .map(|_| ()),
            TargetControlRequest::Observe { .. } | TargetControlRequest::Delete { .. } => {
                contract.check(&target, request_assignment(request))
            }
        }
        .map_err(|_| GuestTargetError::SessionUnavailable)
    }
}

/// The assignment one target-control request carries.
fn request_assignment(request: &TargetControlRequest) -> &TargetControlAssignment {
    match request {
        TargetControlRequest::Realize(request) => request.assignment(),
        TargetControlRequest::Observe { assignment }
        | TargetControlRequest::Delete { assignment }
        | TargetControlRequest::Adopt { assignment } => assignment,
    }
}
