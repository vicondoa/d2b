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

use d2b_contracts::v3::{
    identity::{ResourceName, ResourceTypeName},
    zone_routing::ZoneLabelId,
};
use d2b_zone_routing::engine::{ZoneRelayAdmission, ZoneRelayRequest, ZoneRouteEngine};

pub use d2b_contracts::v3::zone_routing::ZoneRouteFailClosedReason;

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
/// There is deliberately no grant field on this type. A Provider states where
/// it wants to go; it never states that it is allowed to relay. The grants
/// are a separate argument that only the local RBAC engine produces.
#[derive(Clone)]
pub struct ProviderForwardRequest {
    identity: SessionIdentity,
    target: ForwardTarget,
    next_hop: ZoneLabelId,
    arrived_remaining_hops: u32,
    zone_link_connected: bool,
    offers_attachment: bool,
}

impl ProviderForwardRequest {
    /// A forward request with refusing defaults for connectivity.
    pub const fn new(
        identity: SessionIdentity,
        target: ForwardTarget,
        next_hop: ZoneLabelId,
        arrived_remaining_hops: u32,
    ) -> Self {
        Self {
            identity,
            target,
            next_hop,
            arrived_remaining_hops,
            zone_link_connected: false,
            offers_attachment: false,
        }
    }

    /// Record that the route-selected uplink is established.
    #[must_use]
    pub fn with_zone_link_connected(mut self, connected: bool) -> Self {
        self.zone_link_connected = connected;
        self
    }

    /// Record that the inbound frame offered a descriptor attachment.
    #[must_use]
    pub fn with_attachment_offer(mut self, offers_attachment: bool) -> Self {
        self.offers_attachment = offers_attachment;
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
        self.arrived_remaining_hops
    }
}

impl std::fmt::Debug for ProviderForwardRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderForwardRequest")
            .field("arrived_remaining_hops", &self.arrived_remaining_hops)
            .field("zone_link_connected", &self.zone_link_connected)
            .field("offers_attachment", &self.offers_attachment)
            .finish_non_exhaustive()
    }
}

/// The two independent decisions the local RBAC engine reached for one hop.
///
/// The `relay` decision is evaluated against the authenticated inbound Zone
/// transport subject and the route-selected next hop. The target decision is
/// evaluated against the immutable ResourceType, name, and target verb. One
/// never supplies the other, and the only constructors are
/// [`LocalHopGrants::denied`] and [`LocalHopGrants::evaluated`], both of which
/// take the answers the local engine already produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalHopGrants {
    relay_granted: bool,
    target_verb_granted: bool,
}

impl LocalHopGrants {
    /// The refusing default, used when policy state is missing.
    pub const fn denied() -> Self {
        Self {
            relay_granted: false,
            target_verb_granted: false,
        }
    }

    /// Record two independently evaluated local decisions.
    pub const fn evaluated(relay_granted: bool, target_verb_granted: bool) -> Self {
        Self {
            relay_granted,
            target_verb_granted,
        }
    }

    /// Whether the local engine granted `relay` for this next hop.
    pub const fn relay_granted(self) -> bool {
        self.relay_granted
    }

    /// Whether the local engine granted the immutable target verb.
    pub const fn target_verb_granted(self) -> bool {
        self.target_verb_granted
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
/// Both grants are required and are evaluated independently by the caller's
/// own RBAC engine. Nothing on `request` can supply either one.
pub fn admit_provider_forward(
    request: &ProviderForwardRequest,
    grants: LocalHopGrants,
) -> Result<ForwardedCall, ZoneRouteFailClosedReason> {
    let mut hop = ZoneRelayRequest::new(request.arrived_remaining_hops);
    hop.relay_granted = grants.relay_granted;
    hop.target_verb_granted = grants.target_verb_granted;
    hop.zone_link_connected = request.zone_link_connected;
    hop.offers_attachment = request.offers_attachment;
    match ZoneRouteEngine::admit_relay_hop(&hop) {
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
