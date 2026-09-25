//! The serving runtime of one Zone's guest-enrollment surface.
//!
//! [`crate::service`] owns the two landed enrollment handlers and performs no
//! I/O; [`crate::enrollment`] owns the runtime-issued single-use admission and
//! knows no peer. This module is the connection runtime that joins them: it
//! reads one accepted transport, asks the allocator's placement lookup for the
//! link identity the peer named, mints the single-use admission from what it
//! found, dispatches to the handler inside the dispatch window, and writes the
//! handler's answer or its named refusal back.
//!
//! # Dispatch is by decoded call, not by position
//!
//! The frozen contract carries no method name on this path: a Guest agent
//! sends the `zone-bootstrap` payload and then the `zone-enroll` payload, and
//! each payload is closed (`deny_unknown_fields`), so exactly one of the two
//! calls decodes from any frame. The runtime dispatches on what decoded; which
//! call is admissible *next* is the enrollment state machine's decision, and
//! its `invalid-transition` refusal is what answers a replayed bootstrap. A
//! frame that decodes as neither call is not a call at all and ends the
//! connection.
//!
//! # Nothing here is authority
//!
//! The placement lookup is the allocator's, supplied by the caller; this
//! module cannot mint, widen, or invent a placement. The admission is minted
//! from the found tuple and consumed by the handler at the point of use, so a
//! tuple that does not admit the frame's own identity is refused by the
//! handler rather than here. An absent placement, a revoked authority, and
//! every handler refusal reach the peer as the contract's own named refusal.
//! The runtime holds no key, no store, no peer credential, and no path.
//!
//! # Bounds
//!
//! One frame is bounded by [`ZONE_ENROLLMENT_FRAME_LIMIT_BYTES`] and one
//! connection by [`ZONE_ENROLLMENT_CALLS_MAX`] calls, so a peer that never
//! completes enrollment cannot hold the runtime in an unbounded exchange. The
//! dispatch window is the server's own ceiling; a call it refuses to admit is
//! dropped with the server's audit record and reported to the caller as
//! [`ZoneEnrollmentServeError::Overloaded`], because the frozen refusal
//! vocabulary carries no capacity reason and inventing one would put a reason
//! on the wire that no handler produced.
//!
//! # The transition and the reply write are one unit
//!
//! Each served call commits its link FSM transition synchronously inside the
//! handler (PSK burn, enrollment record seal) and then writes the encoded
//! reply. The commit and the write are one unit: there is no await between
//! them, so the reply write is the only suspension point after the
//! transition. If the serve task is dropped mid-send, the connection closes
//! and that close is the peer's only signal that the transition was
//! committed; the caller must not cancel the task between the commit and
//! the write completing.
//!
//! # What this runtime does not serve
//!
//! Enrollment is the whole of its surface. A connection whose enrollment
//! completes is returned to the caller with its transport untouched, because
//! what that connection carries next is the enrolled session's, not this
//! module's.

use std::sync::Arc;

use d2b_bus::session::ZoneLinkState;
use d2b_contracts_resource::v3::execution_policy::PrimitiveSpecError;
use d2b_contracts_zone_session::v3::zone_routing::{ZonePath, ZoneTreeEdge};
use d2b_contracts_zone_session::v3::zone_session::{
    MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES, ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall,
    ZoneEnrollReply, ZoneEnrollmentIdentity, ZoneEnrollmentRefusal,
};
use d2b_session::{OwnedTransport, TransportPacket};

use crate::enrollment::{
    ENROLLMENT_ADMISSION_LIFETIME_MS_DEFAULT, ZoneEnrollmentAuthority, ZoneEnrollmentExpectation,
};
use crate::service::redacted_debug;
use crate::service::{
    ZoneBootstrapRequest, ZoneDispatchAdmission, ZoneEnrollRequest, ZoneServiceAuditEvent,
    ZoneServiceMethod, ZoneServiceServer,
};

/// Largest enrollment frame this runtime reads from one peer.
///
/// This is the contract's own payload bound: a call that encodes larger than
/// this could not be one, so accepting more would only widen the buffer a peer
/// could make this runtime hold.
pub const ZONE_ENROLLMENT_FRAME_LIMIT_BYTES: usize = MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES;

/// Enrollment calls one connection may present before the runtime gives up.
///
/// A refused call leaves the link where it was, so without a bound a peer
/// could keep presenting calls forever. The bound is symmetric with the
/// bounded connect attempts a Guest agent's own base applies to the allocator.
pub const ZONE_ENROLLMENT_CALLS_MAX: u32 = 8;

/// The allocator's placement lookup, keyed by the link identity a peer names.
///
/// The lookup answers one question only: which enrollment, if any, did this
/// allocator authorize for the link this frame names. `None` is an admission
/// this allocator does not hold, and is answered with
/// [`ZoneEnrollmentRefusal::AdmissionAbsent`]. The lookup is a plain closure so
/// the caller keeps ownership of where placements come from - a runtime store,
/// committed rows, or a fixture - and this module keeps none of it.
pub type ZoneEnrollmentPlacements =
    Arc<dyn Fn(&ZoneEnrollmentIdentity) -> Option<ZoneEnrollmentExpectation> + Send + Sync>;

/// Why one enrollment connection ended without enrolling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneEnrollmentServeError {
    /// The transport failed, or the peer closed before enrollment completed.
    Transport,
    /// The peer sent bytes that are not one bounded enrollment call.
    Malformed,
    /// The peer used its bounded call allowance without enrolling.
    Exhausted,
    /// The dispatch window was full, so the call was dropped rather than
    /// served. The server recorded the drop itself.
    Overloaded,
}

impl ZoneEnrollmentServeError {
    /// The stable lower-kebab label of this refusal.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "zone-enrollment-transport",
            Self::Malformed => "zone-enrollment-malformed",
            Self::Exhausted => "zone-enrollment-exhausted",
            Self::Overloaded => "zone-enrollment-overloaded",
        }
    }
}

impl core::fmt::Display for ZoneEnrollmentServeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for ZoneEnrollmentServeError {}

/// The serving runtime of one Zone's enrollment surface.
///
/// One instance belongs to one Zone and is shared by every guest link that
/// Zone is the allocator for: the enrollment state machine of each child link
/// lives in this instance's server, so two connections for one link are
/// serialized by whatever exclusivity the caller gives this value.
pub struct ZoneEnrollmentServer {
    server: ZoneServiceServer,
    authority: ZoneEnrollmentAuthority,
    placements: ZoneEnrollmentPlacements,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

redacted_debug!(ZoneEnrollmentServer);

impl ZoneEnrollmentServer {
    /// Build the runtime over the sealed compiler topology of one Zone.
    ///
    /// `clock` is the single time source for both the admission expiry and the
    /// request timestamp this runtime attaches, so an admission can never be
    /// issued against one clock and consumed against another.
    pub fn new(
        local_root: ZonePath,
        edges: Vec<ZoneTreeEdge>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        placements: ZoneEnrollmentPlacements,
    ) -> Result<Self, PrimitiveSpecError> {
        let server = ZoneServiceServer::new(local_root, edges)?;
        // The compiled default lifetime is within the frozen bounds, checked by
        // `the_compiled_admission_lifetime_is_in_bounds`; the fallible
        // constructor is still handled rather than unwrapped, because an
        // out-of-bounds integer is exactly what `OutOfRange` names.
        let authority = ZoneEnrollmentAuthority::with_lifetime(
            Arc::clone(&clock),
            ENROLLMENT_ADMISSION_LIFETIME_MS_DEFAULT,
        )
        .map_err(|_| PrimitiveSpecError::OutOfRange)?;
        Ok(Self {
            server,
            authority,
            placements,
            clock,
        })
    }

    /// Serve one accepted enrollment connection.
    ///
    /// Serves the peer's bounded bootstrap and enroll calls on the transport
    /// it is given, writing one encoded reply per call, and returns `Ok` once
    /// the peer enrolled. The transport is borrowed, not consumed: the caller
    /// owns what the connection carries after enrollment.
    ///
    /// Each call's link FSM transition (PSK burn, enrollment record seal) is
    /// committed synchronously by the handler, and the encoded reply is
    /// written immediately after as one non-cancellable unit: the reply
    /// write is the only suspension point between the transition and the
    /// peer observing it. If this task is dropped mid-send, the connection
    /// closes and that close is the peer's only signal that the transition
    /// was committed.
    pub async fn serve(
        &mut self,
        transport: &mut dyn OwnedTransport,
    ) -> Result<(), ZoneEnrollmentServeError> {
        let mut calls = 0_u32;
        while calls < ZONE_ENROLLMENT_CALLS_MAX {
            let call = receive_call(transport).await?;
            calls += 1;
            match call {
                EnrollmentCall::Bootstrap(call) => {
                    // The FSM transition and the reply write are one
                    // non-cancellable unit: the handler commits the
                    // transition synchronously, and the encoded reply is
                    // written immediately after, with no await between the
                    // commit and the write.
                    let reply = self.serve_bootstrap(&call)?;
                    let bytes = reply
                        .encode()
                        .map_err(|_| ZoneEnrollmentServeError::Malformed)?;
                    write_reply(transport, bytes).await?;
                }
                EnrollmentCall::Enroll(call) => {
                    // The FSM transition and the reply write are one
                    // non-cancellable unit, exactly as for bootstrap.
                    let reply = self.serve_enroll(&call)?;
                    let enrolled = matches!(reply, ZoneEnrollReply::Enrolled { .. });
                    let bytes = reply
                        .encode()
                        .map_err(|_| ZoneEnrollmentServeError::Malformed)?;
                    write_reply(transport, bytes).await?;
                    if enrolled {
                        return Ok(());
                    }
                }
            }
        }
        Err(ZoneEnrollmentServeError::Exhausted)
    }

    /// The current enrollment state of one sealed child link, when known.
    pub fn link_state(&self, child: &ZonePath) -> Option<ZoneLinkState> {
        self.server.link_state(child)
    }

    /// The bounded audit ring, oldest first.
    pub fn audit_events(&self) -> impl ExactSizeIterator<Item = &ZoneServiceAuditEvent> {
        self.server.audit_events()
    }

    /// Revoke every admission this runtime would issue from now on.
    ///
    /// A revoked runtime keeps serving and refuses every enrollment with
    /// [`ZoneEnrollmentRefusal::PolicyDenial`]: issuance itself refuses, so no
    /// admission is minted for a revoked authority at all.
    pub fn revoke(&self) {
        self.authority.revoke();
    }

    /// Mint one bootstrap request for a decoded call, or the refusal that
    /// stopped it.
    fn bootstrap_request(
        &self,
        call: &ZoneBootstrapCall,
    ) -> Result<ZoneBootstrapRequest, ZoneEnrollmentRefusal> {
        let expected = self.expectation(&call.identity)?;
        let (verifier, evidence) = self.authority.issue(expected.clone())?;
        ZoneBootstrapRequest::new(call.clone(), (self.clock)())
            .with_runtime_admission(verifier, evidence, &expected)
    }

    /// Mint one enroll request for a decoded call, or the refusal that stopped
    /// it.
    fn enroll_request(
        &self,
        call: &ZoneEnrollCall,
    ) -> Result<ZoneEnrollRequest, ZoneEnrollmentRefusal> {
        let expected = self.expectation(&call.identity)?;
        let (verifier, evidence) = self.authority.issue(expected.clone())?;
        ZoneEnrollRequest::new(call.clone(), (self.clock)())
            .with_runtime_admission(verifier, evidence, &expected)
    }

    /// The allocator's placement for one named link, or the closed refusal
    /// that no placement is held.
    fn expectation(
        &self,
        identity: &ZoneEnrollmentIdentity,
    ) -> Result<ZoneEnrollmentExpectation, ZoneEnrollmentRefusal> {
        (self.placements)(identity).ok_or(ZoneEnrollmentRefusal::AdmissionAbsent)
    }

    fn serve_bootstrap(
        &mut self,
        call: &ZoneBootstrapCall,
    ) -> Result<ZoneBootstrapReply, ZoneEnrollmentServeError> {
        let request = match self.bootstrap_request(call) {
            Ok(request) => request,
            Err(reason) => return Ok(ZoneBootstrapReply::Refused { reason }),
        };
        if let ZoneDispatchAdmission::Refused { .. } =
            self.server.begin_dispatch(ZoneServiceMethod::ZoneBootstrap)
        {
            return Err(ZoneEnrollmentServeError::Overloaded);
        }
        let reply = self.server.zone_bootstrap(&request);
        self.server.end_dispatch();
        Ok(reply)
    }

    fn serve_enroll(
        &mut self,
        call: &ZoneEnrollCall,
    ) -> Result<ZoneEnrollReply, ZoneEnrollmentServeError> {
        let request = match self.enroll_request(call) {
            Ok(request) => request,
            Err(reason) => return Ok(ZoneEnrollReply::Refused { reason }),
        };
        if let ZoneDispatchAdmission::Refused { .. } =
            self.server.begin_dispatch(ZoneServiceMethod::ZoneEnroll)
        {
            return Err(ZoneEnrollmentServeError::Overloaded);
        }
        let reply = self.server.zone_enroll(&request);
        self.server.end_dispatch();
        Ok(reply)
    }
}

/// One decoded enrollment call.
enum EnrollmentCall {
    Bootstrap(ZoneBootstrapCall),
    Enroll(ZoneEnrollCall),
}

/// Write one encoded reply, the write half of the commit+write unit.
///
/// The caller has already committed the link FSM transition synchronously
/// (PSK burn, enrollment record seal); this writes the encoded reply as the
/// immediate next step, so the reply write is the only suspension point
/// between the transition and the peer observing it. If the task is dropped
/// mid-send, the connection closes and that close is the peer's only signal
/// that the transition was committed.
async fn write_reply(
    transport: &mut dyn OwnedTransport,
    bytes: Vec<u8>,
) -> Result<(), ZoneEnrollmentServeError> {
    transport
        .send(TransportPacket::new(bytes))
        .await
        .map_err(|_| ZoneEnrollmentServeError::Transport)
}

/// Read one bounded frame and decode it as exactly the call it is.
async fn receive_call(
    transport: &mut dyn OwnedTransport,
) -> Result<EnrollmentCall, ZoneEnrollmentServeError> {
    let frame = transport
        .receive(ZONE_ENROLLMENT_FRAME_LIMIT_BYTES)
        .await
        .map_err(|_| ZoneEnrollmentServeError::Transport)?;
    decode_call(frame.as_bytes()).ok_or(ZoneEnrollmentServeError::Malformed)
}

/// Decode one frame as a bootstrap or an enroll call.
///
/// Both calls are closed, so at most one decodes and an ambiguous frame cannot
/// exist.
fn decode_call(bytes: &[u8]) -> Option<EnrollmentCall> {
    if let Ok(call) = ZoneBootstrapCall::decode(bytes) {
        return Some(EnrollmentCall::Bootstrap(call));
    }
    ZoneEnrollCall::decode(bytes)
        .ok()
        .map(EnrollmentCall::Enroll)
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::ResourceUid;
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZoneLinkControllerGeneration};

    use super::*;

    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("a valid label"))
                .collect(),
        )
        .expect("a valid zone path")
    }

    fn identity() -> ZoneEnrollmentIdentity {
        ZoneEnrollmentIdentity {
            zone_link_uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111")
                .expect("a valid UID"),
            edge: ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"]))
                .expect("a direct child edge"),
            controller_generation: ZoneLinkControllerGeneration::parse("controller-1")
                .expect("a valid generation"),
            reconnect_generation: ReconnectGeneration::new(7).expect("a valid generation"),
            schema_fingerprint: [0x11; 32],
        }
    }

    fn no_placements() -> ZoneEnrollmentPlacements {
        Arc::new(|_| None)
    }

    fn clock() -> Arc<dyn Fn() -> u64 + Send + Sync> {
        Arc::new(|| 1_700_000_000_500)
    }

    #[test]
    fn the_runtime_builds_over_a_sealed_topology_with_no_links_tracked() {
        let server = ZoneEnrollmentServer::new(
            zone(&["k0"]),
            vec![ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"])).expect("a child edge")],
            clock(),
            no_placements(),
        )
        .expect("a sealed topology");
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
    }

    #[test]
    fn a_frame_decodes_as_exactly_one_call() {
        let bootstrap = ZoneBootstrapCall::new(identity(), 1, 300_000, 0)
            .encode()
            .expect("an encodable call");
        assert!(matches!(
            decode_call(&bootstrap),
            Some(EnrollmentCall::Bootstrap(_))
        ));
        let enroll = ZoneEnrollCall::new(identity(), [0x33; 32], 0)
            .encode()
            .expect("an encodable call");
        assert!(matches!(
            decode_call(&enroll),
            Some(EnrollmentCall::Enroll(_))
        ));
        assert!(decode_call(b"not a call").is_none());
    }
}
