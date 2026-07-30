//! Per-hop relay handler.
//!
//! An intermediate Zone forwards an already admitted call exactly one hop.
//! This module owns the bus-side handler for that hop: it derives the two
//! grants from the locally evaluated capability set, refuses carriage that is
//! not adjacent-Zone, drives the Zone routing engine's relay admission through
//! [`RelayHopOracle`], and re-serializes the forwarded envelope with the
//! budget the engine returned.
//!
//! The per-hop rule itself is not restated here. That the relay grant and the
//! target-verb grant are independently required, that an exhausted hop budget
//! and a disconnected uplink refuse, and that a descriptor attachment refuses
//! are all decided by `ZoneRouteEngine::admit_relay_hop`; this module supplies
//! its inputs and consumes its answer.
//!
//! Two structural properties are load bearing:
//!
//! - [`RelayHopGrants`] has no constructor that takes booleans. The only way
//!   to obtain one is from a capability set the local authorizer produced for
//!   this hop, so a peer cannot self-assert a relay grant and no grant is
//!   inherited from a prior hop.
//! - [`RelayHandler`] owns no dedup state and exposes no way to add one. An
//!   intermediate hop forwards; the target Zone is the single dedup owner.

use d2b_contracts::v3::zone_routing::ZoneRouteFailClosedReason;
use d2b_contracts::v3::{AuthenticatedSubjectContext, Locality, zone_routing::ZonePath};
use d2b_resource_api::authz::{PositiveCapabilities, SessionVerb};

use crate::zone_route::ForwardedEnvelope;

/// Structural refusals raised while assembling a relay hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelayError {
    /// `relay` was offered as the forwarded target verb. Relay carries a
    /// call; it is never the verb the call is authorized against, and it
    /// conveys no resource, identity, or local lifecycle authority.
    RelayIsNotATargetVerb,
    /// The verb is not one of the closed set a ZoneLink may forward.
    VerbNotForwardable,
}

/// The closed set of session verbs a ZoneLink hop may forward.
///
/// `connect` establishes the next-hop session, `invoke` and `open-stream`
/// carry runtime service calls, and the two diagnostic verbs bind only to
/// their exact admin service methods. `relay` is absent by construction and
/// `attach`, `cancel`, and `observe` are not forwarded target verbs.
pub const FORWARDABLE_TARGET_VERBS: &[SessionVerb] = &[
    SessionVerb::Connect,
    SessionVerb::Invoke,
    SessionVerb::OpenStream,
    SessionVerb::AuditExport,
    SessionVerb::SupportBundle,
];

/// The two independently required grants for one forwarding hop.
///
/// There is deliberately no constructor from booleans and no setter. Both
/// flags are read out of a capability set that the local authorizer produced
/// for this hop, against this authenticated inbound subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayHopGrants {
    relay_granted: bool,
    target_verb_granted: bool,
    target_verb: SessionVerb,
}

impl RelayHopGrants {
    /// Read both grants out of the locally evaluated capability set.
    ///
    /// The target verb is the immutable verb the original call was authorized
    /// against. Offering `relay` as that verb is refused, which is what makes
    /// a relay grant unable to stand in for the target verb.
    pub fn from_local_capabilities(
        capabilities: &PositiveCapabilities,
        target_verb: SessionVerb,
    ) -> Result<Self, RelayError> {
        if target_verb == SessionVerb::Relay {
            return Err(RelayError::RelayIsNotATargetVerb);
        }
        if !FORWARDABLE_TARGET_VERBS.contains(&target_verb) {
            return Err(RelayError::VerbNotForwardable);
        }
        Ok(Self {
            relay_granted: capabilities.session_verbs.contains(&SessionVerb::Relay),
            target_verb_granted: capabilities.session_verbs.contains(&target_verb),
            target_verb,
        })
    }

    /// Whether the local policy granted `relay` for this hop.
    pub const fn relay_granted(self) -> bool {
        self.relay_granted
    }

    /// Whether the local policy granted the immutable target verb.
    pub const fn target_verb_granted(self) -> bool {
        self.target_verb_granted
    }

    /// The immutable forwarded target verb.
    pub const fn target_verb(self) -> SessionVerb {
        self.target_verb
    }
}

/// The inputs one forwarding question presents to the routing engine.
///
/// The fields correspond one to one with the engine's relay request, so the
/// Zone runtime adapter is a direct field copy and this crate adds no rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayHopInputs {
    /// Hops the inbound frame arrived with.
    pub arrived_remaining_hops: u32,
    /// Whether local policy granted `relay` for this next hop.
    pub relay_granted: bool,
    /// Whether local policy granted the immutable target verb.
    pub target_verb_granted: bool,
    /// Whether the outbound uplink is established.
    pub zone_link_connected: bool,
    /// Whether the inbound frame offered a descriptor attachment.
    pub offers_attachment: bool,
}

/// The routing engine's answer to one forwarding question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayHopAdmission {
    /// The hop may be forwarded with this decremented budget.
    Admitted {
        /// The budget to re-serialize into the forwarded envelope.
        forwarded_remaining_hops: u32,
    },
    /// The hop is refused with a closed reason.
    Denied {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

/// The port through which the bus asks the Zone routing engine to admit a hop.
///
/// The implementation belongs to the Zone runtime and is a direct delegation
/// to `ZoneRouteEngine::admit_relay_hop`.
pub trait RelayHopOracle {
    /// Decide whether this hop may be forwarded, and with what budget.
    fn admit(&self, inputs: RelayHopInputs) -> RelayHopAdmission;
}

/// One forwarding question, as the bus assembles it.
///
/// The authenticated inbound subject is borrowed and never stored. This module
/// cannot construct a subject context and accepts no caller-supplied subject
/// reference, uid, or claim: the registrar remains the sole resolver.
pub struct RelayHopRequest<'a> {
    inbound_subject: &'a AuthenticatedSubjectContext,
    governing_zone_link: &'a ZonePath,
    next_hop_zone: &'a ZonePath,
    target_zone: &'a ZonePath,
    grants: RelayHopGrants,
    zone_link_connected: bool,
    offered_attachments: usize,
}

impl<'a> RelayHopRequest<'a> {
    /// Assemble one forwarding question.
    pub const fn new(
        inbound_subject: &'a AuthenticatedSubjectContext,
        governing_zone_link: &'a ZonePath,
        next_hop_zone: &'a ZonePath,
        target_zone: &'a ZonePath,
        grants: RelayHopGrants,
        zone_link_connected: bool,
        offered_attachments: usize,
    ) -> Self {
        Self {
            inbound_subject,
            governing_zone_link,
            next_hop_zone,
            target_zone,
            grants,
            zone_link_connected,
            offered_attachments,
        }
    }

    /// Borrow the governing ZoneLink's Zone.
    pub const fn governing_zone_link(&self) -> &ZonePath {
        self.governing_zone_link
    }

    /// Borrow the route-selected next hop.
    pub const fn next_hop_zone(&self) -> &ZonePath {
        self.next_hop_zone
    }

    /// Borrow the immutable target Zone.
    pub const fn target_zone(&self) -> &ZonePath {
        self.target_zone
    }

    /// The two independently required grants.
    pub const fn grants(&self) -> RelayHopGrants {
        self.grants
    }
}

impl core::fmt::Debug for RelayHopRequest<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("RelayHopRequest(<redacted>)")
    }
}

/// One admitted hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayedHop {
    envelope: ForwardedEnvelope,
    target_verb: SessionVerb,
}

impl RelayedHop {
    /// Borrow the re-serialized envelope for the next hop.
    pub const fn envelope(&self) -> &ForwardedEnvelope {
        &self.envelope
    }

    /// The immutable forwarded target verb this hop was authorized against.
    pub const fn target_verb(&self) -> SessionVerb {
        self.target_verb
    }
}

/// The per-hop relay handler.
///
/// The handler holds only the routing-engine port. It keeps no dedup ledger,
/// no route table, and no session, so it can neither deduplicate nor reroute.
pub struct RelayHandler<O: RelayHopOracle> {
    oracle: O,
}

impl<O: RelayHopOracle> RelayHandler<O> {
    /// Consume the routing-engine port into one handler.
    pub const fn new(oracle: O) -> Self {
        Self { oracle }
    }

    /// Forward one already admitted call exactly one hop.
    ///
    /// The bus checks only what the engine does not see: that the inbound
    /// carriage is an adjacent-Zone transport, and that the envelope names the
    /// same target Zone this hop was authorized for. Everything else is the
    /// engine's answer.
    pub fn relay(
        &self,
        request: &RelayHopRequest<'_>,
        envelope: &ForwardedEnvelope,
    ) -> Result<RelayedHop, ZoneRouteFailClosedReason> {
        if request.inbound_subject.transport_binding().locality() != Locality::AdjacentZone {
            return Err(ZoneRouteFailClosedReason::RelayDenied);
        }
        if envelope.idempotency().target_zone_path() != request.target_zone {
            return Err(ZoneRouteFailClosedReason::PolicyDenial);
        }

        let inputs = RelayHopInputs {
            arrived_remaining_hops: envelope.remaining_hops(),
            relay_granted: request.grants.relay_granted(),
            target_verb_granted: request.grants.target_verb_granted(),
            zone_link_connected: request.zone_link_connected,
            offers_attachment: request.offered_attachments != 0,
        };
        match self.oracle.admit(inputs) {
            RelayHopAdmission::Denied { reason } => Err(reason),
            RelayHopAdmission::Admitted {
                forwarded_remaining_hops,
            } => Ok(RelayedHop {
                envelope: envelope.forwarded(forwarded_remaining_hops),
                target_verb: request.grants.target_verb(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use d2b_contracts::v3::zone_routing::{ZoneLabelId, ZonePath};
    use d2b_contracts::v3::{
        BindingDigest, EvidenceClass, ReconnectGeneration, ResourceName, ResourceRef,
        ResourceTypeName, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding,
        SessionPurpose, TranscriptHash, TransportBinding,
    };
    use d2b_resource_api::authz::ApiMethod;

    use crate::operations::OperationId;
    use crate::zone_route::{
        ForwardedEnvelope, ForwardedSelector, OpaqueCallToken, PrincipalDigest,
        ZoneLinkIdempotencyKey,
    };

    /// The exact per-hop rule the Zone routing engine owns.
    ///
    /// The test oracle mirrors `ZoneRouteEngine::admit_relay_hop` so the
    /// handler is exercised against the real answers without this crate
    /// depending on the routing crate. The handler under test contains none
    /// of this logic.
    struct EngineOracle;

    impl RelayHopOracle for EngineOracle {
        fn admit(&self, inputs: RelayHopInputs) -> RelayHopAdmission {
            let denied = |reason| RelayHopAdmission::Denied { reason };
            if inputs.offers_attachment {
                return denied(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink);
            }
            if inputs.arrived_remaining_hops == 0 {
                return denied(ZoneRouteFailClosedReason::HopLimitExceeded);
            }
            if !inputs.zone_link_connected {
                return denied(ZoneRouteFailClosedReason::ZoneLinkDisconnected);
            }
            if !inputs.relay_granted {
                return denied(ZoneRouteFailClosedReason::RelayDenied);
            }
            if !inputs.target_verb_granted {
                return denied(ZoneRouteFailClosedReason::PolicyDenial);
            }
            RelayHopAdmission::Admitted {
                forwarded_remaining_hops: inputs.arrived_remaining_hops - 1,
            }
        }
    }

    /// Records the inputs the handler passed to the engine.
    struct RecordingOracle {
        seen: std::cell::RefCell<Vec<RelayHopInputs>>,
    }

    impl RelayHopOracle for RecordingOracle {
        fn admit(&self, inputs: RelayHopInputs) -> RelayHopAdmission {
            self.seen.borrow_mut().push(inputs);
            EngineOracle.admit(inputs)
        }
    }

    /// Build a Zone path from root-first labels.
    ///
    /// The contract type stores labels most specific first, so the readable
    /// root-first spelling used by these tests is reversed here.
    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .rev()
                .map(|label| ZoneLabelId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn reference(resource_type: &str, name: &str) -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse(resource_type).unwrap(),
            ResourceName::parse(name).unwrap(),
        )
    }

    fn subject(locality: Locality) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            reference("Provider", "relay-hop"),
            ResourceUid::parse("00000000-0000-4000-8000-000000000001").unwrap(),
            reference("Zone", "k1"),
            EvidenceClass::EnrolledKk,
            SessionPurpose::parse("remote-zone").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    locality,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([7u8; 32]),
            ),
        )
    }

    fn capabilities(verbs: &[SessionVerb]) -> PositiveCapabilities {
        PositiveCapabilities {
            resources: Vec::new(),
            session_verbs: verbs.iter().copied().collect::<BTreeSet<_>>(),
        }
    }

    fn envelope(remaining_hops: u32) -> ForwardedEnvelope {
        ForwardedEnvelope::seal(
            ZoneLinkIdempotencyKey::new(
                OperationId::parse("op-1").unwrap(),
                OpaqueCallToken::parse("idem-1").unwrap(),
                zone(&["k0"]),
                zone(&["k0", "k1", "k2"]),
                ApiMethod::UpdateSpec,
                PrincipalDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            ),
            ForwardedSelector::named(
                ResourceTypeName::parse("Process").unwrap(),
                ResourceName::parse("worker").unwrap(),
            ),
            OpaqueCallToken::parse("corr-1").unwrap(),
            OpaqueCallToken::parse("trace-1").unwrap(),
            None,
            remaining_hops,
            0,
        )
        .unwrap()
    }

    fn request<'a>(
        inbound: &'a AuthenticatedSubjectContext,
        governing: &'a ZonePath,
        next_hop: &'a ZonePath,
        target: &'a ZonePath,
        grants: RelayHopGrants,
        connected: bool,
        attachments: usize,
    ) -> RelayHopRequest<'a> {
        RelayHopRequest::new(
            inbound,
            governing,
            next_hop,
            target,
            grants,
            connected,
            attachments,
        )
    }

    fn grants(verbs: &[SessionVerb]) -> RelayHopGrants {
        RelayHopGrants::from_local_capabilities(&capabilities(verbs), SessionVerb::Invoke).unwrap()
    }

    #[test]
    fn a_hop_with_both_grants_forwards_the_call_with_the_engine_decremented_budget() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        let hop = handler
            .relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &envelope(14),
            )
            .unwrap();
        assert_eq!(hop.envelope().remaining_hops(), 13);
        assert_eq!(hop.target_verb(), SessionVerb::Invoke);
    }

    #[test]
    fn a_missing_relay_grant_refuses_the_hop() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::RelayDenied)
        );
    }

    #[test]
    fn a_missing_target_verb_refuses_the_hop_even_with_relay() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay]),
                    true,
                    0,
                ),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn relay_alone_grants_no_resource_or_local_lifecycle_verb() {
        let relay_only = capabilities(&[SessionVerb::Relay]);
        for verb in [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::OpenStream,
            SessionVerb::AuditExport,
            SessionVerb::SupportBundle,
        ] {
            let derived = RelayHopGrants::from_local_capabilities(&relay_only, verb).unwrap();
            assert!(derived.relay_granted());
            assert!(
                !derived.target_verb_granted(),
                "a relay grant must never supply the target verb"
            );
        }
        assert!(relay_only.resources.is_empty());
    }

    #[test]
    fn relay_cannot_be_presented_as_the_forwarded_target_verb() {
        assert_eq!(
            RelayHopGrants::from_local_capabilities(
                &capabilities(&[SessionVerb::Relay]),
                SessionVerb::Relay
            ),
            Err(RelayError::RelayIsNotATargetVerb)
        );
    }

    #[test]
    fn a_verb_outside_the_forwardable_set_is_refused() {
        for verb in [
            SessionVerb::Attach,
            SessionVerb::Cancel,
            SessionVerb::Observe,
        ] {
            assert_eq!(
                RelayHopGrants::from_local_capabilities(
                    &capabilities(&[SessionVerb::Relay, verb]),
                    verb
                ),
                Err(RelayError::VerbNotForwardable)
            );
        }
    }

    #[test]
    fn a_self_asserted_relay_claim_cannot_produce_a_grant() {
        // A peer that names itself a relay purpose, on an enrolled ZoneLink,
        // still derives no relay grant: the flags come only from the local
        // capability set, and an empty set yields nothing.
        let derived =
            RelayHopGrants::from_local_capabilities(&capabilities(&[]), SessionVerb::Invoke)
                .unwrap();
        assert!(!derived.relay_granted());
        assert!(!derived.target_verb_granted());

        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        assert_eq!(
            handler.relay(
                &request(&inbound, &governing, &next_hop, &target, derived, true, 0),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::RelayDenied)
        );
    }

    #[test]
    fn an_exhausted_hop_budget_refuses_the_hop() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        let exhausted = envelope(1).forwarded(0);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &exhausted,
            ),
            Err(ZoneRouteFailClosedReason::HopLimitExceeded)
        );
    }

    #[test]
    fn an_offered_descriptor_attachment_refuses_the_hop() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    1,
                ),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink)
        );
    }

    #[test]
    fn a_disconnected_outbound_uplink_refuses_the_hop() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    false,
                    0,
                ),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
    }

    #[test]
    fn carriage_that_is_not_adjacent_zone_is_refused_before_the_engine_is_asked() {
        let handler = RelayHandler::new(RecordingOracle {
            seen: std::cell::RefCell::new(Vec::new()),
        });
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        for locality in [Locality::Local, Locality::Remote] {
            let inbound = subject(locality);
            assert_eq!(
                handler.relay(
                    &request(
                        &inbound,
                        &governing,
                        &next_hop,
                        &target,
                        grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                        true,
                        0,
                    ),
                    &envelope(14),
                ),
                Err(ZoneRouteFailClosedReason::RelayDenied)
            );
        }
        assert!(handler.oracle.seen.borrow().is_empty());
    }

    #[test]
    fn an_envelope_naming_another_target_zone_is_refused() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let other_target = zone(&["k0", "k1", "k9"]);
        assert_eq!(
            handler.relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &other_target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &envelope(14),
            ),
            Err(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn the_handler_forwards_the_engine_its_own_inputs_and_adds_no_rule() {
        let handler = RelayHandler::new(RecordingOracle {
            seen: std::cell::RefCell::new(Vec::new()),
        });
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        handler
            .relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &envelope(14),
            )
            .unwrap();
        assert_eq!(
            handler.oracle.seen.borrow().as_slice(),
            &[RelayHopInputs {
                arrived_remaining_hops: 14,
                relay_granted: true,
                target_verb_granted: true,
                zone_link_connected: true,
                offers_attachment: false,
            }]
        );
    }

    #[test]
    fn an_intermediate_hop_forwards_a_repeated_call_rather_than_deduplicating_it() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        let call = envelope(14);
        for _ in 0..3 {
            let hop = handler
                .relay(
                    &request(
                        &inbound,
                        &governing,
                        &next_hop,
                        &target,
                        grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                        true,
                        0,
                    ),
                    &call,
                )
                .unwrap();
            assert_eq!(hop.envelope().remaining_hops(), 13);
            assert_eq!(hop.envelope().selector(), call.selector());
        }
    }

    #[test]
    fn a_relayed_hop_preserves_the_selector_and_every_call_identifier() {
        let handler = RelayHandler::new(EngineOracle);
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        let call = envelope(14);
        let hop = handler
            .relay(
                &request(
                    &inbound,
                    &governing,
                    &next_hop,
                    &target,
                    grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
                    true,
                    0,
                ),
                &call,
            )
            .unwrap();
        assert_eq!(hop.envelope().selector(), call.selector());
        assert_eq!(hop.envelope().idempotency(), call.idempotency());
        assert_eq!(hop.envelope().correlation_id(), call.correlation_id());
        assert_eq!(hop.envelope().trace_id(), call.trace_id());
        assert_eq!(
            hop.envelope().watch_after_revision(),
            call.watch_after_revision()
        );
    }

    #[test]
    fn a_relay_request_debug_reveals_no_subject_or_zone() {
        let inbound = subject(Locality::AdjacentZone);
        let governing = zone(&["k0", "k1"]);
        let next_hop = zone(&["k0", "k1", "k2"]);
        let target = zone(&["k0", "k1", "k2"]);
        let assembled = request(
            &inbound,
            &governing,
            &next_hop,
            &target,
            grants(&[SessionVerb::Relay, SessionVerb::Invoke]),
            true,
            0,
        );
        assert_eq!(
            format!("{assembled:?}"),
            "RelayHopRequest(<redacted>)".to_owned()
        );
    }
}
