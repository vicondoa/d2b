//! Forwarding admission for a Provider call that leaves the local Zone.
//!
//! D040 freezes `relay` as a session verb distinct from the target verb:
//! every forwarding hop independently requires `relay` plus the target verb,
//! preserves the immutable target exactly, and grants no CRUD, identity
//! mapping, capability widening, or local lifecycle authority. Self-asserted
//! grants fail closed.
//!
//! This module is the Provider-side entry to that rule. It carries no policy
//! of its own: the hop decision itself is
//! [`ZoneRouteEngine::admit_relay_hop`], which this crate reuses rather than
//! restates.

use d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId;
use d2b_bus::session::{RouteAdmissionEvidence, RouteAdmissionVerifier};
use d2b_contracts_resource::v3::{
    ResourceName,
    ResourceTypeName,
};
use d2b_zone_routing::engine::{
    ZoneRelayAdmission, ZoneRelayRequest, ZoneRouteAdmission,
    ZoneRouteAdmissionExpectation, ZoneRouteEngine,
};

pub use d2b_contracts_zone_session::v3::zone_routing::ZoneRouteFailClosedReason;

use crate::session::SessionIdentity;

/// The immutable target a forwarded call names.
///
/// A hop may not rewrite any of this. A named method keeps its exact resource
/// name; a nameless `List` or `Watch` keeps `None` and is authorized against
/// the hop's own bounded selector.
#[derive(Clone, PartialEq, Eq)]
pub struct ForwardTarget {
    resource_type: ResourceTypeName,
    resource_name: Option<ResourceName>,
}

impl ForwardTarget {
    /// A target naming exactly one resource.
    pub const fn named(resource_type: ResourceTypeName, resource_name: ResourceName) -> Self {
        Self {
            resource_type,
            resource_name: Some(resource_name),
        }
    }

    /// A nameless target, for `List` and `Watch`.
    pub const fn nameless(resource_type: ResourceTypeName) -> Self {
        Self {
            resource_type,
            resource_name: None,
        }
    }

    /// The immutable ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// The immutable resource name, when the target is named.
    pub const fn resource_name(&self) -> Option<&ResourceName> {
        self.resource_name.as_ref()
    }
}

impl std::fmt::Debug for ForwardTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardTarget")
            .field("named", &self.resource_name.is_some())
            .finish_non_exhaustive()
    }
}

/// What an authenticated Provider session asks a hop to forward.
///
/// A request carries the two runtime-issued admissions only after the local
/// runtime has verified them. There is no caller-populated policy or
/// connectivity claim and no grant boolean that a Provider can assert.
pub struct ProviderForwardRequest {
    identity: SessionIdentity,
    target: ForwardTarget,
    next_hop: ZoneLabelId,
    hop: ZoneRelayRequest,
}

impl ProviderForwardRequest {
    /// A forward request with no route admissions.
    pub fn new(
        identity: SessionIdentity,
        target: ForwardTarget,
        next_hop: ZoneLabelId,
        arrived_remaining_hops: u32,
    ) -> Self {
        let hop = ZoneRelayRequest::new(arrived_remaining_hops)
            .with_forward_binding(identity.zone().clone(), next_hop.clone());
        Self {
            identity,
            target,
            next_hop,
            hop,
        }
    }

    /// Attach the independently verified target and relay admissions.
    pub fn with_admissions(
        mut self,
        target_admission: ZoneRouteAdmission,
        relay_admission: ZoneRouteAdmission,
    ) -> Self {
        self.hop = self
            .hop
            .with_admissions(target_admission, relay_admission);
        self
    }

    /// Consume and verify the target and relay admissions for this hop.
    pub fn with_runtime_admissions(
        self,
        target_verifier: RouteAdmissionVerifier,
        target_evidence: RouteAdmissionEvidence,
        target_expected: &ZoneRouteAdmissionExpectation,
        relay_verifier: RouteAdmissionVerifier,
        relay_evidence: RouteAdmissionEvidence,
        relay_expected: &ZoneRouteAdmissionExpectation,
    ) -> Result<Self, ZoneRouteFailClosedReason> {
        Ok(self.with_admissions(
            ZoneRouteAdmission::verify(target_verifier, target_evidence, target_expected)?,
            ZoneRouteAdmission::verify(relay_verifier, relay_evidence, relay_expected)?,
        ))
    }

    /// Record that the inbound frame offered a descriptor attachment.
    #[must_use]
    pub fn with_attachment_offer(mut self, offers_attachment: bool) -> Self {
        self.hop = self.hop.with_attachment_offer(offers_attachment);
        self
    }

    /// The authenticated inbound identity.
    pub const fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    /// The immutable target.
    pub const fn target(&self) -> &ForwardTarget {
        &self.target
    }

    /// The route-selected next hop.
    pub const fn next_hop(&self) -> &ZoneLabelId {
        &self.next_hop
    }

    /// The hop budget the inbound frame arrived with.
    pub const fn arrived_remaining_hops(&self) -> u32 {
        self.hop.arrived_remaining_hops()
    }
}

impl std::fmt::Debug for ProviderForwardRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderForwardRequest")
            .field("arrived_remaining_hops", &self.arrived_remaining_hops())
            .finish_non_exhaustive()
    }
}

/// One admitted forward: the unchanged target and the decremented budget.
#[derive(Clone, PartialEq, Eq)]
pub struct ForwardedCall {
    target: ForwardTarget,
    next_hop: ZoneLabelId,
    forwarded_remaining_hops: u32,
}

impl ForwardedCall {
    /// The target, preserved exactly as it arrived.
    pub const fn target(&self) -> &ForwardTarget {
        &self.target
    }

    /// The next hop the call is re-serialized towards.
    pub const fn next_hop(&self) -> &ZoneLabelId {
        &self.next_hop
    }

    /// The hop budget to re-serialize into the forwarded frame.
    pub const fn forwarded_remaining_hops(&self) -> u32 {
        self.forwarded_remaining_hops
    }
}

impl std::fmt::Debug for ForwardedCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardedCall")
            .field("forwarded_remaining_hops", &self.forwarded_remaining_hops)
            .finish_non_exhaustive()
    }
}

/// Decide whether one hop may forward an authenticated Provider call.
///
/// Both runtime-issued admissions are required and are independently verified
/// before the hop can be forwarded. Nothing on `request` can supply either
/// one.
pub fn admit_provider_forward(
    request: &ProviderForwardRequest,
) -> Result<ForwardedCall, ZoneRouteFailClosedReason> {
    match ZoneRouteEngine::admit_relay_hop(&request.hop) {
        ZoneRelayAdmission::Admitted {
            forwarded_remaining_hops,
        } => Ok(ForwardedCall {
            target: request.target.clone(),
            next_hop: request.next_hop.clone(),
            forwarded_remaining_hops,
        }),
        ZoneRelayAdmission::Denied { reason } => Err(reason),
    }
}
