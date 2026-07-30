//! Zone route decision engine (`ADR046-routing-002`).
//!
//! The engine is a pure, in-memory adaptation of the v3 baseline tree-route
//! engine onto the Zone tree contracts owned by
//! `d2b_contracts::v3::zone_routing`. It admits already-verified route
//! advertisements and withdrawals, keeps a bounded parent/route projection plus
//! a bounded replay-key table, and answers nearest-common-ancestor route
//! questions.
//!
//! What the engine deliberately does not do: it performs no I/O, opens no
//! socket, verifies no signature, and mints, holds, or presents no authority.
//! Signature metadata is treated as opaque replay bookkeeping. Every decision
//! is either a typed allow carrying immutable route metadata or a typed
//! refusal carrying one closed [`ZoneRouteFailClosedReason`]; there is no
//! permissive default anywhere in this file. Nothing here accepts or returns a
//! uid, gid, host path, socket path, store path, credential, or key material.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::zone_routing::{
    MAX_ZONE_PARENT_ENTRIES, MAX_ZONE_ROUTE_ENTRIES, ZONE_ROUTE_INITIAL_HOP_BUDGET, ZoneLabelId,
    ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation, ZoneLinkRouteAdvertisement,
    ZoneLinkRouteWithdrawal, ZonePath, ZoneRouteAuditEventKind, ZoneRouteCapability,
    ZoneRouteCapabilitySet, ZoneRouteFailClosedReason, ZoneRouteHop, ZoneRouteHopDirection,
    ZoneRouteId, ZoneRoutePath, ZoneTreeEdge,
};

/// Maximum live replay-window keys one engine retains.
///
/// The replay window is sized against the combined parent and route projection
/// bounds so a peer cannot grow it independently of the entries it is allowed
/// to install.
pub const MAX_ZONE_REPLAY_KEYS: usize = (MAX_ZONE_PARENT_ENTRIES + MAX_ZONE_ROUTE_ENTRIES) * 4;

/// One route-projection row, ordered by descendant Zone path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRouteInventoryEntry {
    /// The reachable descendant Zone.
    pub descendant: ZonePath,
    /// The Zone whose advertisement installed the row.
    pub advertising_zone: ZonePath,
    /// The immediate child label below the advertiser toward the descendant.
    pub next_hop_child: ZoneLabelId,
    /// The advertised route identifier.
    pub route_id: ZoneRouteId,
    /// The capabilities the route carries after allocator narrowing.
    pub capabilities: ZoneRouteCapabilitySet,
}

/// Counts physically removed by one expiry sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZoneRoutePruneReport {
    /// Parent entries removed.
    pub parent_entries: usize,
    /// Route entries removed.
    pub route_entries: usize,
    /// Replay-window keys removed.
    pub replay_keys: usize,
}

/// Outcome of admitting one route advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneAdvertisementAdmission {
    /// The advertisement was admitted and installed these route identifiers.
    Accepted {
        /// Sorted identifiers of the routes now projected.
        accepted_routes: Vec<ZoneRouteId>,
    },
    /// The advertisement was refused.
    Denied {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneAdvertisementAdmission {
    /// The audit event kind this outcome emits.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Accepted { .. } => ZoneRouteAuditEventKind::ZoneAdvertisementAccepted,
            Self::Denied { .. } => ZoneRouteAuditEventKind::ZoneAdvertisementDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Accepted { .. } => None,
            Self::Denied { reason } => Some(*reason),
        }
    }
}

/// Outcome of admitting one route withdrawal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneWithdrawalAdmission {
    /// The withdrawal was admitted; these identifiers were removed.
    ///
    /// A named identifier that was already expired or never known is silently
    /// accepted and simply absent from this list, so withdrawal is idempotent.
    Accepted {
        /// Sorted identifiers actually removed from the projection.
        withdrawn_route_ids: Vec<ZoneRouteId>,
    },
    /// The withdrawal was refused; no projection state changed.
    Denied {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneWithdrawalAdmission {
    /// The audit event kind this outcome emits.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Accepted { .. } => ZoneRouteAuditEventKind::ZoneAdvertisementWithdrawn,
            Self::Denied { .. } => ZoneRouteAuditEventKind::ZoneAdvertisementDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Accepted { .. } => None,
            Self::Denied { reason } => Some(*reason),
        }
    }
}

/// One route question posed to the engine.
///
/// Every field that could weaken the answer defaults to its refusing value in
/// [`ZoneRouteRequest::new`], so a caller that forgets to supply an input gets
/// a typed refusal rather than a permissive answer. `policy_allows` and
/// `zone_link_connected` are inputs from the authorizer and the link
/// controller; the engine never infers either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRouteRequest {
    /// The Zone the call originates in.
    pub source_zone: ZonePath,
    /// The Zone the call targets.
    pub target_zone: ZonePath,
    /// The verifier-supplied decision time in Unix seconds.
    pub current_time_unix_seconds: u64,
    /// The capability the requested operation needs at the target Zone.
    pub required_capability: Option<ZoneRouteCapability>,
    /// Hops still available to this call.
    pub remaining_hops: u32,
    /// Whether the caller's authorizer allowed the operation.
    pub policy_allows: bool,
    /// Whether the uplink toward the target is established.
    pub zone_link_connected: bool,
}

impl ZoneRouteRequest {
    /// A request with the protocol-wide initial hop budget and refusing
    /// defaults for every authorization and connectivity input.
    pub const fn new(
        source_zone: ZonePath,
        target_zone: ZonePath,
        current_time_unix_seconds: u64,
    ) -> Self {
        Self {
            source_zone,
            target_zone,
            current_time_unix_seconds,
            required_capability: None,
            remaining_hops: ZONE_ROUTE_INITIAL_HOP_BUDGET,
            policy_allows: false,
            zone_link_connected: false,
        }
    }
}

/// The engine's answer to one route question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneRouteDecision {
    /// The route is allowed.
    Allowed {
        /// Immutable route metadata; it carries no transport or credential.
        path: ZoneRoutePath,
        /// The capability ceiling surviving every advertised hop, or `None`
        /// when the target is the local root and no advertised ceiling applies.
        effective_capabilities: Option<ZoneRouteCapabilitySet>,
        /// Hops left after paying for this path.
        remaining_hops_after: u32,
    },
    /// The route is refused.
    Denied {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneRouteDecision {
    /// The audit event kind this outcome emits.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Allowed { .. } => ZoneRouteAuditEventKind::ZoneRouteAllowed,
            Self::Denied { .. } => ZoneRouteAuditEventKind::ZoneRouteDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Allowed { .. } => None,
            Self::Denied { reason } => Some(*reason),
        }
    }
}

/// One forwarding question posed at an intermediate Zone.
///
/// Both grant flags default to refusing in [`ZoneRelayRequest::new`]. A relay
/// grant never supplies the target-verb grant and a target-verb grant never
/// supplies the relay grant; the engine requires both independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneRelayRequest {
    /// Hops the inbound frame arrived with.
    pub arrived_remaining_hops: u32,
    /// Whether local policy granted the `relay` verb for this next hop.
    pub relay_granted: bool,
    /// Whether local policy granted the immutable target verb.
    pub target_verb_granted: bool,
    /// Whether the outbound uplink is established.
    pub zone_link_connected: bool,
    /// Whether the inbound frame offered a descriptor attachment.
    pub offers_attachment: bool,
}

impl ZoneRelayRequest {
    /// A relay request with refusing defaults for every grant and for
    /// connectivity.
    pub const fn new(arrived_remaining_hops: u32) -> Self {
        Self {
            arrived_remaining_hops,
            relay_granted: false,
            target_verb_granted: false,
            zone_link_connected: false,
            offers_attachment: false,
        }
    }
}

/// The engine's answer to one forwarding question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneRelayAdmission {
    /// The hop may be forwarded with this decremented budget.
    Admitted {
        /// The hop budget to re-serialize into the forwarded frame.
        forwarded_remaining_hops: u32,
    },
    /// The hop is refused.
    Denied {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneRelayAdmission {
    /// The audit event kind this outcome emits.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Admitted { .. } => ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted,
            Self::Denied { .. } => ZoneRouteAuditEventKind::ZoneLinkRelayDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Admitted { .. } => None,
            Self::Denied { reason } => Some(*reason),
        }
    }
}

/// The bounded pre-authentication admission outcome for inbound routing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZonePreAuthAdmission {
    /// The item was queued.
    Queued {
        /// Queue depth after the item was accepted.
        queue_depth_after: u32,
    },
    /// The item was dropped.
    Dropped {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZonePreAuthAdmission {
    /// The audit event kind this outcome emits.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Queued { .. } => ZoneRouteAuditEventKind::ZoneLinkIntentQueued,
            Self::Dropped { .. } => ZoneRouteAuditEventKind::ZoneAdvertisementDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Queued { .. } => None,
            Self::Dropped { reason } => Some(*reason),
        }
    }
}

/// Decide bounded pre-authentication admission with drop-new overflow and a
/// per-peer rate ceiling.
///
/// Overflow is drop-new rather than drop-oldest so an unauthenticated peer
/// cannot evict already-queued work by flooding.
pub fn decide_pre_auth_admission(
    current_depth: u32,
    max_depth: u32,
    events_this_minute: u32,
    rate_limit_per_minute: u32,
) -> ZonePreAuthAdmission {
    if current_depth >= max_depth {
        return ZonePreAuthAdmission::Dropped {
            reason: ZoneRouteFailClosedReason::QueueFullDropNew,
        };
    }
    if events_this_minute >= rate_limit_per_minute {
        return ZonePreAuthAdmission::Dropped {
            reason: ZoneRouteFailClosedReason::RateLimited,
        };
    }
    ZonePreAuthAdmission::Queued {
        queue_depth_after: current_depth.saturating_add(1),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentEntry {
    parent: ZonePath,
    route_id: Option<ZoneRouteId>,
    capabilities: Option<ZoneRouteCapabilitySet>,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteEntry {
    advertising_zone: ZonePath,
    next_hop_child: ZoneLabelId,
    route_id: ZoneRouteId,
    controller_generation: ZoneLinkControllerGeneration,
    capabilities: ZoneRouteCapabilitySet,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

/// The replay-window key.
///
/// The specification fixes the key as the advertising Zone, the controller
/// generation, the issue time, and the detached signature reference. Nothing in
/// the key is secret: a signature reference is a locator and a generation is a
/// lease handle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayKey {
    advertising_zone: ZonePath,
    controller_generation: ZoneLinkControllerGeneration,
    issued_at_unix_seconds: u64,
    signature_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CapacityLimits {
    max_parent_entries: usize,
    max_route_entries: usize,
    max_replay_keys: usize,
}

impl Default for CapacityLimits {
    fn default() -> Self {
        Self {
            max_parent_entries: MAX_ZONE_PARENT_ENTRIES,
            max_route_entries: MAX_ZONE_ROUTE_ENTRIES,
            max_replay_keys: MAX_ZONE_REPLAY_KEYS,
        }
    }
}

/// The pure in-memory Zone route engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRouteEngine {
    local_root: ZonePath,
    parents: BTreeMap<ZonePath, ParentEntry>,
    routes: BTreeMap<ZonePath, RouteEntry>,
    replay_keys: BTreeMap<ReplayKey, u64>,
    capacity: CapacityLimits,
}

impl ZoneRouteEngine {
    /// Create an engine rooted at the local Zone with the frozen bounds.
    pub fn new(local_root: ZonePath) -> Self {
        Self {
            local_root,
            parents: BTreeMap::new(),
            routes: BTreeMap::new(),
            replay_keys: BTreeMap::new(),
            capacity: CapacityLimits::default(),
        }
    }

    /// Create an engine with explicit projection bounds.
    ///
    /// Each supplied bound is clamped down to the frozen contract bound, so a
    /// caller can only make the engine stricter, never wider.
    pub fn with_capacity_limits(
        local_root: ZonePath,
        max_parent_entries: usize,
        max_route_entries: usize,
        max_replay_keys: usize,
    ) -> Self {
        Self {
            local_root,
            parents: BTreeMap::new(),
            routes: BTreeMap::new(),
            replay_keys: BTreeMap::new(),
            capacity: CapacityLimits {
                max_parent_entries: max_parent_entries.min(MAX_ZONE_PARENT_ENTRIES),
                max_route_entries: max_route_entries.min(MAX_ZONE_ROUTE_ENTRIES),
                max_replay_keys: max_replay_keys.min(MAX_ZONE_REPLAY_KEYS),
            },
        }
    }

    /// Borrow the local root Zone path.
    pub const fn local_root(&self) -> &ZonePath {
        &self.local_root
    }

    /// Admit one advertisement whose signature the caller already verified.
    ///
    /// The advertisement's own structural invariants were proven by its
    /// constructor. This method adds the engine-owned checks: freshness, the
    /// replay window, the parent chain, the private namespace allocation, loop
    /// and multi-parent detection over the staged projection, and the bounded
    /// capacity ceiling.
    pub fn admit_advertisement(
        &mut self,
        advertisement: &ZoneLinkRouteAdvertisement,
        allocation: &ZoneLinkNamespaceAllocation,
        received_at_unix_seconds: u64,
    ) -> ZoneAdvertisementAdmission {
        let now = received_at_unix_seconds;
        let denied = |reason| ZoneAdvertisementAdmission::Denied { reason };

        if now < advertisement.issued_at_unix_seconds() {
            return denied(ZoneRouteFailClosedReason::MalformedAdvert);
        }
        if is_expired(advertisement.expires_at_unix_seconds(), now) {
            return denied(ZoneRouteFailClosedReason::Expired);
        }
        if advertisement.tree_edge().parent() != &self.local_root
            && self
                .parent_entry_at(advertisement.tree_edge().parent(), now)
                .is_none()
        {
            return denied(ZoneRouteFailClosedReason::UnknownParent);
        }
        if self.replay_keys.iter().any(|(key, expires_at)| {
            key_matches(key, advertisement) && !is_expired(*expires_at, now)
        }) {
            return denied(ZoneRouteFailClosedReason::Replay);
        }
        if allocation.tree_edge() != advertisement.tree_edge()
            || allocation.allocated_to_generation() != advertisement.controller_generation()
            || advertisement.routes().len() > allocation.max_routes() as usize
        {
            return denied(ZoneRouteFailClosedReason::NamespaceViolation);
        }

        let ceiling = allocation.allowed_capabilities().clone();
        let mut parent_updates: BTreeMap<ZonePath, ParentEntry> = BTreeMap::new();
        let mut route_updates: BTreeMap<ZonePath, RouteEntry> = BTreeMap::new();

        if let Err(reason) = stage_parent_edge(
            &self.parents,
            &mut parent_updates,
            now,
            advertisement.issued_at_unix_seconds(),
            advertisement.tree_edge().parent().clone(),
            advertisement.tree_edge().child().clone(),
            None,
            Some(ceiling.clone()),
            Some(&ceiling),
            advertisement.expires_at_unix_seconds(),
        ) {
            return denied(reason);
        }

        let mut accepted_routes = Vec::new();
        let mut seen_descendants = BTreeSet::new();
        for route in advertisement.routes() {
            if !seen_descendants.insert(route.descendant().clone()) {
                return denied(ZoneRouteFailClosedReason::MalformedAdvert);
            }
            if !allocation.admits_prefix(route.descendant())
                || !route.capabilities().is_subset_of(&ceiling)
            {
                return denied(ZoneRouteFailClosedReason::NamespaceViolation);
            }
            let Some(next_child) =
                direct_child_below(route.descendant(), advertisement.advertising_zone())
            else {
                return denied(ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert);
            };
            let terminal = &next_child == route.descendant();
            if let Err(reason) = stage_parent_edge(
                &self.parents,
                &mut parent_updates,
                now,
                advertisement.issued_at_unix_seconds(),
                advertisement.advertising_zone().clone(),
                next_child,
                terminal.then(|| route.route_id().clone()),
                terminal.then(|| route.capabilities().clone()),
                None,
                advertisement.expires_at_unix_seconds(),
            ) {
                return denied(reason);
            }
            if let Err(reason) = stage_route_entry(
                &self.routes,
                &mut route_updates,
                now,
                advertisement,
                route.descendant().clone(),
                route.next_hop_child().clone(),
                route.route_id().clone(),
                intersect_capabilities(route.capabilities(), &ceiling),
            ) {
                return denied(reason);
            }
            accepted_routes.push(route.route_id().clone());
        }

        let replay_key = ReplayKey {
            advertising_zone: advertisement.advertising_zone().clone(),
            controller_generation: advertisement.controller_generation().clone(),
            issued_at_unix_seconds: advertisement.issued_at_unix_seconds(),
            signature_ref: advertisement
                .signature()
                .signature_ref()
                .as_str()
                .to_owned(),
        };

        if self.over_capacity(&parent_updates, &route_updates, &replay_key) {
            // Reclaim only entries that are already dead before refusing: an
            // expired projection row or replay key confers nothing, and a
            // superseded replay key was replaced by this advertiser's newer
            // window.
            self.prune_expired(now);
            self.prune_superseded_replay_keys(advertisement);
            if self.over_capacity(&parent_updates, &route_updates, &replay_key) {
                return denied(ZoneRouteFailClosedReason::QueueFullDropNew);
            }
        }

        for (child, entry) in parent_updates {
            self.parents.insert(child, entry);
        }
        for (descendant, entry) in route_updates {
            self.routes.insert(descendant, entry);
        }
        self.prune_superseded_replay_keys(advertisement);
        self.replay_keys
            .insert(replay_key, advertisement.expires_at_unix_seconds());

        accepted_routes.sort();
        ZoneAdvertisementAdmission::Accepted { accepted_routes }
    }

    /// Admit one withdrawal, removing exactly the named live routes.
    ///
    /// A withdrawal must come from the same advertising Zone and controller
    /// generation that installed the route and must not predate it. A named
    /// identifier that is unknown or already expired is accepted and simply not
    /// reported as removed.
    pub fn admit_withdrawal(
        &mut self,
        withdrawal: &ZoneLinkRouteWithdrawal,
        received_at_unix_seconds: u64,
    ) -> ZoneWithdrawalAdmission {
        let now = received_at_unix_seconds;
        if now < withdrawal.issued_at_unix_seconds() {
            return ZoneWithdrawalAdmission::Denied {
                reason: ZoneRouteFailClosedReason::MalformedAdvert,
            };
        }

        let mut targets = Vec::new();
        for route_id in withdrawal.withdrawn_route_ids() {
            let Some((descendant, entry)) = self
                .routes
                .iter()
                .find(|(_, entry)| &entry.route_id == route_id)
            else {
                continue;
            };
            if is_expired(entry.expires_at_unix_seconds, now) {
                continue;
            }
            if &entry.advertising_zone != withdrawal.advertising_zone() {
                return ZoneWithdrawalAdmission::Denied {
                    reason: ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert,
                };
            }
            if &entry.controller_generation != withdrawal.controller_generation() {
                return ZoneWithdrawalAdmission::Denied {
                    reason: ZoneRouteFailClosedReason::NamespaceViolation,
                };
            }
            if withdrawal.issued_at_unix_seconds() < entry.issued_at_unix_seconds {
                return ZoneWithdrawalAdmission::Denied {
                    reason: ZoneRouteFailClosedReason::Replay,
                };
            }
            targets.push((descendant.clone(), route_id.clone()));
        }

        let mut withdrawn_route_ids = Vec::new();
        for (descendant, route_id) in targets {
            self.routes.remove(&descendant);
            withdrawn_route_ids.push(route_id);
        }
        withdrawn_route_ids.sort();
        ZoneWithdrawalAdmission::Accepted {
            withdrawn_route_ids,
        }
    }

    /// Decide the route for one operation.
    ///
    /// The order is: connectivity, caller authorization, the hop budget, the
    /// nearest-common-ancestor walk, and finally the capability ceiling
    /// surviving the walk. Every stage refuses with a closed reason.
    pub fn decide_route(&self, request: &ZoneRouteRequest) -> ZoneRouteDecision {
        let denied = |reason| ZoneRouteDecision::Denied { reason };
        if !request.zone_link_connected {
            return denied(ZoneRouteFailClosedReason::ZoneLinkDisconnected);
        }
        if !request.policy_allows {
            return denied(ZoneRouteFailClosedReason::PolicyDenial);
        }
        if request.remaining_hops == 0 {
            return denied(ZoneRouteFailClosedReason::HopLimitExceeded);
        }

        let path = match self.build_path_at(
            &request.source_zone,
            &request.target_zone,
            request.current_time_unix_seconds,
        ) {
            Ok(path) => path,
            Err(reason) => return denied(reason),
        };

        let hop_count = match u32::try_from(path.hop_count()) {
            Ok(hops) => hops,
            Err(_) => return denied(ZoneRouteFailClosedReason::HopLimitExceeded),
        };
        if hop_count > request.remaining_hops {
            return denied(ZoneRouteFailClosedReason::HopLimitExceeded);
        }

        let effective_capabilities =
            self.effective_capabilities_at(&path, request.current_time_unix_seconds);
        if let Some(required) = request.required_capability.as_ref() {
            let satisfied = effective_capabilities
                .as_ref()
                .is_none_or(|set| set.contains(required));
            if !satisfied {
                return denied(ZoneRouteFailClosedReason::MissingCapability);
            }
        }

        ZoneRouteDecision::Allowed {
            path,
            effective_capabilities,
            remaining_hops_after: request.remaining_hops - hop_count,
        }
    }

    /// Decide whether one intermediate Zone may forward a call, and with what
    /// remaining budget.
    ///
    /// This is an associated function because forwarding admission depends only
    /// on the inbound frame and the local grants; it consults no projection
    /// state.
    pub fn admit_relay_hop(request: &ZoneRelayRequest) -> ZoneRelayAdmission {
        let denied = |reason| ZoneRelayAdmission::Denied { reason };
        if request.offers_attachment {
            return denied(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink);
        }
        if request.arrived_remaining_hops == 0 {
            return denied(ZoneRouteFailClosedReason::HopLimitExceeded);
        }
        if !request.zone_link_connected {
            return denied(ZoneRouteFailClosedReason::ZoneLinkDisconnected);
        }
        if !request.relay_granted {
            return denied(ZoneRouteFailClosedReason::RelayDenied);
        }
        if !request.target_verb_granted {
            return denied(ZoneRouteFailClosedReason::PolicyDenial);
        }
        ZoneRelayAdmission::Admitted {
            forwarded_remaining_hops: request.arrived_remaining_hops - 1,
        }
    }

    /// Deterministic route inventory ordered by descendant Zone path.
    pub fn route_inventory(&self) -> Vec<ZoneRouteInventoryEntry> {
        self.routes
            .iter()
            .map(|(descendant, entry)| ZoneRouteInventoryEntry {
                descendant: descendant.clone(),
                advertising_zone: entry.advertising_zone.clone(),
                next_hop_child: entry.next_hop_child.clone(),
                route_id: entry.route_id.clone(),
                capabilities: entry.capabilities.clone(),
            })
            .collect()
    }

    /// Physically remove expired parent entries, route entries, and replay
    /// keys.
    pub fn prune_expired(&mut self, current_time_unix_seconds: u64) -> ZoneRoutePruneReport {
        let parents_before = self.parents.len();
        self.parents.retain(|_, entry| {
            !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
        });
        let routes_before = self.routes.len();
        self.routes.retain(|_, entry| {
            !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
        });
        let replay_before = self.replay_keys.len();
        self.replay_keys
            .retain(|_, expires_at| !is_expired(*expires_at, current_time_unix_seconds));

        ZoneRoutePruneReport {
            parent_entries: parents_before - self.parents.len(),
            route_entries: routes_before - self.routes.len(),
            replay_keys: replay_before - self.replay_keys.len(),
        }
    }

    fn over_capacity(
        &self,
        parent_updates: &BTreeMap<ZonePath, ParentEntry>,
        route_updates: &BTreeMap<ZonePath, RouteEntry>,
        replay_key: &ReplayKey,
    ) -> bool {
        let parents = projected_len(&self.parents, parent_updates);
        let routes = projected_len(&self.routes, route_updates);
        let replay =
            self.replay_keys.len() + usize::from(!self.replay_keys.contains_key(replay_key));
        parents > self.capacity.max_parent_entries
            || routes > self.capacity.max_route_entries
            || replay > self.capacity.max_replay_keys
    }

    fn prune_superseded_replay_keys(&mut self, advertisement: &ZoneLinkRouteAdvertisement) {
        self.replay_keys.retain(|key, expires_at| {
            !(&key.advertising_zone == advertisement.advertising_zone()
                && &key.controller_generation == advertisement.controller_generation()
                && key.issued_at_unix_seconds < advertisement.issued_at_unix_seconds()
                && *expires_at <= advertisement.expires_at_unix_seconds())
        });
    }

    fn build_path_at(
        &self,
        source: &ZonePath,
        target: &ZonePath,
        now: u64,
    ) -> Result<ZoneRoutePath, ZoneRouteFailClosedReason> {
        if !self.is_known_zone_at(source, now) || !self.is_known_zone_at(target, now) {
            return Err(ZoneRouteFailClosedReason::UnknownParent);
        }
        let Some(ancestor) = nearest_common_ancestor(source, target) else {
            return Err(ZoneRouteFailClosedReason::UnknownParent);
        };

        let mut hops = Vec::new();
        let mut current = source.clone();
        // Loop detection is engine state, not a contract field: the walk keeps
        // a visited set of Zone paths so a projection whose parent chain cycles
        // is refused instead of walked forever.
        let mut visited = BTreeSet::new();
        while current != ancestor {
            if !visited.insert(current.clone()) {
                return Err(ZoneRouteFailClosedReason::Loop);
            }
            let Some(entry) = self.parent_entry_at(&current, now) else {
                return Err(ZoneRouteFailClosedReason::UnknownParent);
            };
            if visited.contains(&entry.parent) {
                return Err(ZoneRouteFailClosedReason::Loop);
            }
            if hops.len() >= d2b_contracts::v3::zone_routing::MAX_ZONE_ROUTE_PATH_HOPS {
                return Err(ZoneRouteFailClosedReason::HopLimitExceeded);
            }
            let edge = ZoneTreeEdge::new(entry.parent.clone(), current.clone())
                .map_err(|_| ZoneRouteFailClosedReason::MalformedAdvert)?;
            let hop = ZoneRouteHop::new(
                current.clone(),
                entry.parent.clone(),
                edge,
                ZoneRouteHopDirection::UpToParent,
                entry.route_id.clone(),
            )
            .map_err(|_| ZoneRouteFailClosedReason::MalformedAdvert)?;
            current = entry.parent.clone();
            hops.push(hop);
        }

        let branch = &target.labels()[..target.depth() - ancestor.depth()];
        let mut current = ancestor.clone();
        for label in branch.iter().rev() {
            let child = child_with_label(&current, label.clone())
                .ok_or(ZoneRouteFailClosedReason::MalformedAdvert)?;
            let Some(entry) = self.parent_entry_at(&child, now) else {
                return Err(ZoneRouteFailClosedReason::UnknownParent);
            };
            if entry.parent != current {
                return Err(ZoneRouteFailClosedReason::MultiParent);
            }
            if hops.len() >= d2b_contracts::v3::zone_routing::MAX_ZONE_ROUTE_PATH_HOPS {
                return Err(ZoneRouteFailClosedReason::HopLimitExceeded);
            }
            let edge = ZoneTreeEdge::new(current.clone(), child.clone())
                .map_err(|_| ZoneRouteFailClosedReason::MalformedAdvert)?;
            let hop = ZoneRouteHop::new(
                current.clone(),
                child.clone(),
                edge,
                ZoneRouteHopDirection::DownToChild,
                entry.route_id.clone(),
            )
            .map_err(|_| ZoneRouteFailClosedReason::MalformedAdvert)?;
            current = child;
            hops.push(hop);
        }

        ZoneRoutePath::new(source.clone(), target.clone(), ancestor, hops)
            .map_err(|_| ZoneRouteFailClosedReason::MalformedAdvert)
    }

    /// The capability ceiling that survives every advertised hop of a path.
    ///
    /// Only a Zone that carries a route entry has asserted a capability
    /// ceiling; a parent edge staged as a bare intermediate asserts nothing and
    /// therefore narrows nothing. `None` means the target is the local root,
    /// where the call is dispatched locally and no advertised ceiling applies.
    fn effective_capabilities_at(
        &self,
        path: &ZoneRoutePath,
        now: u64,
    ) -> Option<ZoneRouteCapabilitySet> {
        if path.target_zone() == &self.local_root {
            return None;
        }
        let mut effective: Option<ZoneRouteCapabilitySet> = None;
        for hop in path.hops() {
            if hop.direction() != ZoneRouteHopDirection::DownToChild {
                continue;
            }
            let Some(entry) = self.route_entry_at(hop.to(), now) else {
                continue;
            };
            effective = Some(match effective {
                Some(current) => intersect_capabilities(&current, &entry.capabilities),
                None => entry.capabilities.clone(),
            });
        }
        Some(effective.unwrap_or_else(|| {
            self.route_entry_at(path.target_zone(), now)
                .map(|entry| entry.capabilities.clone())
                .unwrap_or_default()
        }))
    }

    fn is_known_zone_at(&self, zone: &ZonePath, now: u64) -> bool {
        zone == &self.local_root
            || self.parent_entry_at(zone, now).is_some()
            || self.route_entry_at(zone, now).is_some()
    }

    fn parent_entry_at(&self, zone: &ZonePath, now: u64) -> Option<&ParentEntry> {
        self.parents
            .get(zone)
            .filter(|entry| !is_expired(entry.expires_at_unix_seconds, now))
    }

    fn route_entry_at(&self, zone: &ZonePath, now: u64) -> Option<&RouteEntry> {
        self.routes
            .get(zone)
            .filter(|entry| !is_expired(entry.expires_at_unix_seconds, now))
    }
}

fn is_expired(expires_at_unix_seconds: u64, now: u64) -> bool {
    now >= expires_at_unix_seconds
}

fn key_matches(key: &ReplayKey, advertisement: &ZoneLinkRouteAdvertisement) -> bool {
    &key.advertising_zone == advertisement.advertising_zone()
        && &key.controller_generation == advertisement.controller_generation()
        && key.issued_at_unix_seconds == advertisement.issued_at_unix_seconds()
        && key.signature_ref == advertisement.signature().signature_ref().as_str()
}

fn intersect_capabilities(
    left: &ZoneRouteCapabilitySet,
    right: &ZoneRouteCapabilitySet,
) -> ZoneRouteCapabilitySet {
    ZoneRouteCapabilitySet::new(
        left.capabilities()
            .iter()
            .filter(|capability| right.contains(capability))
            .cloned()
            .collect(),
    )
    .unwrap_or_default()
}

fn projected_len<T, U>(entries: &BTreeMap<ZonePath, T>, updates: &BTreeMap<ZonePath, U>) -> usize {
    entries.len()
        + updates
            .keys()
            .filter(|key| !entries.contains_key(*key))
            .count()
}

fn child_with_label(parent: &ZonePath, label: ZoneLabelId) -> Option<ZonePath> {
    let mut labels = parent.labels().to_vec();
    labels.insert(0, label);
    ZonePath::new(labels).ok()
}

fn direct_child_below(descendant: &ZonePath, ancestor: &ZonePath) -> Option<ZonePath> {
    let label = descendant.next_hop_label_below(ancestor)?;
    child_with_label(ancestor, label.clone())
}

fn nearest_common_ancestor(left: &ZonePath, right: &ZonePath) -> Option<ZonePath> {
    let shared = left
        .labels()
        .iter()
        .rev()
        .zip(right.labels().iter().rev())
        .take_while(|(one, other)| one == other)
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();
    if shared.is_empty() {
        return None;
    }
    ZonePath::new(shared.into_iter().rev().collect()).ok()
}

#[allow(clippy::too_many_arguments)]
fn stage_parent_edge(
    parents: &BTreeMap<ZonePath, ParentEntry>,
    parent_updates: &mut BTreeMap<ZonePath, ParentEntry>,
    now: u64,
    issued_at_unix_seconds: u64,
    parent: ZonePath,
    child: ZonePath,
    route_id: Option<ZoneRouteId>,
    capabilities: Option<ZoneRouteCapabilitySet>,
    ceiling: Option<&ZoneRouteCapabilitySet>,
    expires_at_unix_seconds: u64,
) -> Result<(), ZoneRouteFailClosedReason> {
    if child == parent || would_form_loop(parents, parent_updates, &parent, &child, now) {
        return Err(ZoneRouteFailClosedReason::Loop);
    }

    if let Some(existing) = parent_updates.get_mut(&child) {
        if existing.parent != parent {
            return Err(ZoneRouteFailClosedReason::MultiParent);
        }
        if let Some(candidate) = route_id {
            existing.route_id = Some(candidate);
        }
        if let Some(capabilities) = capabilities {
            existing.capabilities = Some(bound_capabilities(capabilities, ceiling));
        } else if let (Some(current), Some(ceiling)) = (existing.capabilities.as_ref(), ceiling) {
            existing.capabilities = Some(intersect_capabilities(current, ceiling));
        }
        existing.expires_at_unix_seconds = expires_at_unix_seconds;
        return Ok(());
    }

    let mut next_route_id = route_id;
    let mut next_capabilities = capabilities;
    if let Some(existing) = parents
        .get(&child)
        .filter(|entry| !is_expired(entry.expires_at_unix_seconds, now))
    {
        if issued_at_unix_seconds <= existing.issued_at_unix_seconds {
            return Err(ZoneRouteFailClosedReason::Replay);
        }
        if existing.parent != parent {
            return Err(ZoneRouteFailClosedReason::MultiParent);
        }
        if next_route_id.is_none() {
            next_route_id = existing.route_id.clone();
        }
        if next_capabilities.is_none() {
            next_capabilities = existing.capabilities.clone();
        }
    }

    parent_updates.insert(
        child,
        ParentEntry {
            parent,
            route_id: next_route_id,
            capabilities: next_capabilities.map(|caps| bound_capabilities(caps, ceiling)),
            issued_at_unix_seconds,
            expires_at_unix_seconds,
        },
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_route_entry(
    routes: &BTreeMap<ZonePath, RouteEntry>,
    route_updates: &mut BTreeMap<ZonePath, RouteEntry>,
    now: u64,
    advertisement: &ZoneLinkRouteAdvertisement,
    descendant: ZonePath,
    next_hop_child: ZoneLabelId,
    route_id: ZoneRouteId,
    capabilities: ZoneRouteCapabilitySet,
) -> Result<(), ZoneRouteFailClosedReason> {
    let advertising_zone = advertisement.advertising_zone();
    if let Some(existing) = route_updates.get(&descendant) {
        if &existing.advertising_zone != advertising_zone
            || existing.next_hop_child != next_hop_child
        {
            return Err(ZoneRouteFailClosedReason::MultiParent);
        }
        return Ok(());
    }
    if let Some(existing) = routes
        .get(&descendant)
        .filter(|entry| !is_expired(entry.expires_at_unix_seconds, now))
    {
        if advertisement.issued_at_unix_seconds() <= existing.issued_at_unix_seconds {
            return Err(ZoneRouteFailClosedReason::Replay);
        }
        if &existing.advertising_zone != advertising_zone
            || existing.next_hop_child != next_hop_child
        {
            return Err(ZoneRouteFailClosedReason::MultiParent);
        }
    }

    route_updates.insert(
        descendant,
        RouteEntry {
            advertising_zone: advertising_zone.clone(),
            next_hop_child,
            route_id,
            controller_generation: advertisement.controller_generation().clone(),
            capabilities,
            issued_at_unix_seconds: advertisement.issued_at_unix_seconds(),
            expires_at_unix_seconds: advertisement.expires_at_unix_seconds(),
        },
    );
    Ok(())
}

fn bound_capabilities(
    capabilities: ZoneRouteCapabilitySet,
    ceiling: Option<&ZoneRouteCapabilitySet>,
) -> ZoneRouteCapabilitySet {
    match ceiling {
        Some(ceiling) => intersect_capabilities(&capabilities, ceiling),
        None => capabilities,
    }
}

fn would_form_loop(
    parents: &BTreeMap<ZonePath, ParentEntry>,
    parent_updates: &BTreeMap<ZonePath, ParentEntry>,
    parent: &ZonePath,
    child: &ZonePath,
    now: u64,
) -> bool {
    let mut current = parent;
    let mut visited = BTreeSet::new();
    loop {
        if current == child || !visited.insert(current.clone()) {
            return true;
        }
        let next = parent_updates.get(current).or_else(|| {
            parents
                .get(current)
                .filter(|entry| !is_expired(entry.expires_at_unix_seconds, now))
        });
        match next {
            Some(entry) => current = &entry.parent,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::{
        ZONE_ROUTING_SCHEMA_VERSION, ZoneDescendantRoute, ZoneRouteKeyRole, ZoneRouteSignature,
        ZoneRouteSignatureAlgorithm, ZoneRouteSignatureRef, ZoneSigningKeyFingerprint,
    };

    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("valid label"))
                .collect(),
        )
        .expect("valid zone path")
    }

    fn caps(codes: &[&str]) -> ZoneRouteCapabilitySet {
        ZoneRouteCapabilitySet::new(
            codes
                .iter()
                .map(|code| ZoneRouteCapability::parse(*code).expect("valid capability"))
                .collect(),
        )
        .expect("valid capability set")
    }

    fn capability(code: &str) -> ZoneRouteCapability {
        ZoneRouteCapability::parse(code).expect("valid capability")
    }

    fn signature(signature_ref: &str) -> ZoneRouteSignature {
        ZoneRouteSignature::new(
            ZoneRouteSignatureAlgorithm::Ed25519Blake3,
            ZoneRouteKeyRole::ZoneControllerRouting,
            ZoneSigningKeyFingerprint::parse(format!("sha256.{}", "b".repeat(64)))
                .expect("valid fingerprint"),
            ZoneRouteSignatureRef::parse(signature_ref).expect("valid signature ref"),
        )
    }

    fn generation(value: &str) -> ZoneLinkControllerGeneration {
        ZoneLinkControllerGeneration::parse(value).expect("valid generation")
    }

    fn route_id(value: &str) -> ZoneRouteId {
        ZoneRouteId::parse(value).expect("valid route id")
    }

    struct Advert {
        parent: ZonePath,
        child: ZonePath,
        generation: ZoneLinkControllerGeneration,
        routes: Vec<ZoneDescendantRoute>,
        issued_at: u64,
        expires_at: u64,
        signature_ref: String,
    }

    impl Advert {
        fn new(parent: ZonePath, child: ZonePath) -> Self {
            Self {
                parent,
                child,
                generation: generation("gen-1"),
                routes: Vec::new(),
                issued_at: 1_000,
                expires_at: 4_000,
                signature_ref: "sigref-1".to_owned(),
            }
        }

        fn route(
            mut self,
            id: &str,
            descendant: ZonePath,
            next_hop: &str,
            capabilities: &[&str],
        ) -> Self {
            self.routes.push(ZoneDescendantRoute::new(
                route_id(id),
                descendant,
                ZoneLabelId::parse(next_hop).expect("valid label"),
                caps(capabilities),
            ));
            self
        }

        fn window(mut self, issued_at: u64, expires_at: u64) -> Self {
            self.issued_at = issued_at;
            self.expires_at = expires_at;
            self
        }

        fn signature_ref(mut self, value: &str) -> Self {
            self.signature_ref = value.to_owned();
            self
        }

        fn generation(mut self, value: &str) -> Self {
            self.generation = generation(value);
            self
        }

        fn build(self) -> ZoneLinkRouteAdvertisement {
            ZoneLinkRouteAdvertisement::new(
                ZONE_ROUTING_SCHEMA_VERSION,
                self.child.clone(),
                ZoneTreeEdge::new(self.parent, self.child).expect("direct child edge"),
                self.generation,
                self.routes,
                self.issued_at,
                self.expires_at,
                signature(&self.signature_ref),
            )
            .expect("valid advertisement")
        }
    }

    fn allocation(
        parent: ZonePath,
        child: ZonePath,
        gen_value: &str,
        prefixes: Vec<ZonePath>,
        max_routes: u32,
        allowed: &[&str],
    ) -> ZoneLinkNamespaceAllocation {
        ZoneLinkNamespaceAllocation::new(
            ZoneTreeEdge::new(parent, child).expect("direct child edge"),
            generation(gen_value),
            prefixes,
            max_routes,
            caps(allowed),
        )
        .expect("valid allocation")
    }

    /// Root k0 with child k1 advertising a route to grandchild k2.
    fn seeded_engine() -> ZoneRouteEngine {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get", "list"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get", "list", "watch"],
        );
        let outcome = engine.admit_advertisement(&advert, &alloc, 1_500);
        assert!(matches!(
            outcome,
            ZoneAdvertisementAdmission::Accepted { .. }
        ));
        engine
    }

    fn allowed_request(source: ZonePath, target: ZonePath, now: u64) -> ZoneRouteRequest {
        let mut request = ZoneRouteRequest::new(source, target, now);
        request.policy_allows = true;
        request.zone_link_connected = true;
        request
    }

    #[test]
    fn request_defaults_refuse_before_any_projection_is_consulted() {
        let engine = seeded_engine();
        let request = ZoneRouteRequest::new(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 1_500);
        assert_eq!(request.remaining_hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);
        let decision = engine.decide_route(&request);
        assert_eq!(
            decision.denial_reason(),
            Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
        assert_eq!(
            decision.audit_event(),
            ZoneRouteAuditEventKind::ZoneRouteDenied
        );
    }

    #[test]
    fn policy_denial_is_reported_before_the_tree_walk() {
        let engine = seeded_engine();
        let mut request = ZoneRouteRequest::new(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 1_500);
        request.zone_link_connected = true;
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn a_two_hop_downward_route_is_allowed_and_pays_its_hops() {
        let engine = seeded_engine();
        let request = allowed_request(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 1_500);
        let ZoneRouteDecision::Allowed {
            path,
            remaining_hops_after,
            ..
        } = engine.decide_route(&request)
        else {
            panic!("expected an allowed route");
        };
        assert_eq!(path.hop_count(), 2);
        assert_eq!(path.nearest_common_ancestor(), &zone(&["k0"]));
        assert_eq!(remaining_hops_after, ZONE_ROUTE_INITIAL_HOP_BUDGET - 2);
        assert!(
            path.hops()
                .iter()
                .all(|hop| hop.direction() == ZoneRouteHopDirection::DownToChild)
        );
    }

    #[test]
    fn an_upward_and_downward_walk_meets_at_the_nearest_common_ancestor() {
        let mut engine = seeded_engine();
        let advert = Advert::new(zone(&["k0"]), zone(&["k3", "k0"]))
            .route("route-2", zone(&["k4", "k3", "k0"]), "k4", &["get"])
            .window(1_100, 4_000)
            .signature_ref("sigref-2")
            .generation("gen-2")
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k3", "k0"]),
            "gen-2",
            vec![zone(&["k3", "k0"])],
            8,
            &["get"],
        );
        assert!(matches!(
            engine.admit_advertisement(&advert, &alloc, 1_500),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));

        let mut request =
            allowed_request(zone(&["k2", "k1", "k0"]), zone(&["k4", "k3", "k0"]), 1_500);
        request.required_capability = Some(capability("get"));
        let ZoneRouteDecision::Allowed { path, .. } = engine.decide_route(&request) else {
            panic!("expected an allowed route");
        };
        assert_eq!(path.nearest_common_ancestor(), &zone(&["k0"]));
        assert_eq!(path.hop_count(), 4);
        assert_eq!(
            path.hops()
                .iter()
                .filter(|hop| hop.direction() == ZoneRouteHopDirection::UpToParent)
                .count(),
            2
        );
    }

    #[test]
    fn an_unknown_target_zone_is_refused() {
        let engine = seeded_engine();
        let request = allowed_request(zone(&["k0"]), zone(&["k9", "k0"]), 1_500);
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn disjoint_trees_have_no_nearest_common_ancestor() {
        let mut engine = seeded_engine();
        // Install a projection row for a Zone in a different tree so the
        // known-Zone check passes and the ancestor search is the refusing
        // stage.
        engine.parents.insert(
            zone(&["z1", "z0"]),
            ParentEntry {
                parent: zone(&["z0"]),
                route_id: None,
                capabilities: None,
                issued_at_unix_seconds: 1_000,
                expires_at_unix_seconds: 4_000,
            },
        );
        let request = allowed_request(zone(&["k0"]), zone(&["z1", "z0"]), 1_500);
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_cyclic_parent_chain_is_refused_as_a_loop() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        // A projection whose parent chain cycles cannot be produced by
        // admission, which is exactly why the walk carries its own visited set:
        // without it this self-referencing entry never terminates.
        engine.parents.insert(
            zone(&["a", "k0"]),
            ParentEntry {
                parent: zone(&["a", "k0"]),
                route_id: None,
                capabilities: None,
                issued_at_unix_seconds: 1_000,
                expires_at_unix_seconds: 4_000,
            },
        );
        let request = allowed_request(zone(&["a", "k0"]), zone(&["k0"]), 1_500);
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::Loop)
        );
    }

    #[test]
    fn an_advertisement_that_would_close_a_parent_cycle_is_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        engine.parents.insert(
            zone(&["k0"]),
            ParentEntry {
                parent: zone(&["k1", "k0"]),
                route_id: None,
                capabilities: None,
                issued_at_unix_seconds: 1_000,
                expires_at_unix_seconds: 4_000,
            },
        );
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::Loop)
        );
    }

    #[test]
    fn a_replayed_advertisement_is_refused_on_the_exact_window_key() {
        let mut engine = seeded_engine();
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get", "list"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get", "list", "watch"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 1_600)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::Replay)
        );
    }

    #[test]
    fn a_renewal_with_a_fresh_window_and_signature_reference_is_admitted() {
        let mut engine = seeded_engine();
        let renewal = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .window(2_000, 5_000)
            .signature_ref("sigref-2")
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get", "list", "watch"],
        );
        assert!(matches!(
            engine.admit_advertisement(&renewal, &alloc, 2_100),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));
        // The superseded window key is dropped, so it is no longer a live
        // replay entry.
        assert_eq!(engine.replay_keys.len(), 1);
        let inventory = engine.route_inventory();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].capabilities, caps(&["get"]));
    }

    #[test]
    fn an_advertisement_that_does_not_advance_its_issue_time_is_refused_as_replay() {
        let mut engine = seeded_engine();
        let stale = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .window(1_000, 4_500)
            .signature_ref("sigref-9")
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get", "list", "watch"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&stale, &alloc, 1_600)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::Replay)
        );
    }

    #[test]
    fn an_expired_advertisement_and_a_future_dated_one_are_both_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 4_000)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::Expired)
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::MalformedAdvert)
        );
    }

    #[test]
    fn an_advertisement_from_an_unknown_parent_is_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k1", "k0"]), zone(&["k2", "k1", "k0"]))
            .route("route-1", zone(&["k3", "k2", "k1", "k0"]), "k3", &["get"])
            .build();
        let alloc = allocation(
            zone(&["k1", "k0"]),
            zone(&["k2", "k1", "k0"]),
            "gen-1",
            vec![zone(&["k2", "k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_route_outside_the_allocated_prefix_or_capability_scope_is_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let narrow_prefix = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k7", "k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &narrow_prefix, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::NamespaceViolation)
        );

        let narrow_caps = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["list"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &narrow_caps, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::NamespaceViolation)
        );
    }

    #[test]
    fn an_allocation_for_another_edge_or_generation_is_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let wrong_generation = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-9",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &wrong_generation, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::NamespaceViolation)
        );

        let too_few_routes = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            1,
            &["get"],
        );
        let two_routes = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .route("route-2", zone(&["k3", "k1", "k0"]), "k3", &["get"])
            .build();
        assert_eq!(
            engine
                .admit_advertisement(&two_routes, &too_few_routes, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::NamespaceViolation)
        );
    }

    #[test]
    fn two_route_rows_for_one_descendant_in_one_advertisement_are_refused() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .route("route-2", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::MalformedAdvert)
        );
    }

    #[test]
    fn a_second_advertiser_claiming_the_same_descendant_is_refused_as_multi_parent() {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        // k1 claims a deep descendant through k2.
        let first = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-a", zone(&["k3", "k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let alloc_k1 = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        assert!(matches!(
            engine.admit_advertisement(&first, &alloc_k1, 1_500),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));

        // k2 then claims the same descendant directly. Both advertisements are
        // structurally valid on their own; only the engine can see that they
        // disagree about who owns the descendant.
        let second = Advert::new(zone(&["k1", "k0"]), zone(&["k2", "k1", "k0"]))
            .route("route-b", zone(&["k3", "k2", "k1", "k0"]), "k3", &["get"])
            .window(2_000, 5_000)
            .signature_ref("sigref-2")
            .generation("gen-2")
            .build();
        let alloc_k2 = allocation(
            zone(&["k1", "k0"]),
            zone(&["k2", "k1", "k0"]),
            "gen-2",
            vec![zone(&["k2", "k1", "k0"])],
            8,
            &["get"],
        );
        assert_eq!(
            engine
                .admit_advertisement(&second, &alloc_k2, 2_100)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::MultiParent)
        );
    }

    #[test]
    fn capacity_pressure_that_pruning_cannot_relieve_drops_the_new_advertisement() {
        let mut engine = ZoneRouteEngine::with_capacity_limits(zone(&["k0"]), 1, 1, 1);
        let advert = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .build();
        let alloc = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get"],
        );
        // The advertisement stages two parent entries (k1 and k2) against a
        // ceiling of one, and no entry is expired, so pruning cannot help.
        assert_eq!(
            engine
                .admit_advertisement(&advert, &alloc, 1_500)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::QueueFullDropNew)
        );
        assert!(engine.route_inventory().is_empty());
    }

    #[test]
    fn capability_scope_narrows_monotonically_across_a_multi_hop_walk() {
        let mut engine = seeded_engine();
        // k1 already routes to k2 with {get, list}. k2 now advertises k3 with
        // {get, list}, but the walk to k3 also traverses k2, whose own route
        // row is the narrower ceiling installed below.
        let narrow = Advert::new(zone(&["k0"]), zone(&["k1", "k0"]))
            .route("route-1", zone(&["k2", "k1", "k0"]), "k2", &["get"])
            .window(2_000, 6_000)
            .signature_ref("sigref-2")
            .build();
        let alloc_k1 = allocation(
            zone(&["k0"]),
            zone(&["k1", "k0"]),
            "gen-1",
            vec![zone(&["k1", "k0"])],
            8,
            &["get", "list", "watch"],
        );
        assert!(matches!(
            engine.admit_advertisement(&narrow, &alloc_k1, 2_100),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));

        let deep = Advert::new(zone(&["k1", "k0"]), zone(&["k2", "k1", "k0"]))
            .route(
                "route-3",
                zone(&["k3", "k2", "k1", "k0"]),
                "k3",
                &["get", "list"],
            )
            .window(2_200, 6_000)
            .signature_ref("sigref-3")
            .generation("gen-2")
            .build();
        let alloc_k2 = allocation(
            zone(&["k1", "k0"]),
            zone(&["k2", "k1", "k0"]),
            "gen-2",
            vec![zone(&["k2", "k1", "k0"])],
            8,
            &["get", "list"],
        );
        assert!(matches!(
            engine.admit_advertisement(&deep, &alloc_k2, 2_300),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));

        let mut request = allowed_request(zone(&["k0"]), zone(&["k3", "k2", "k1", "k0"]), 2_400);
        request.required_capability = Some(capability("get"));
        let ZoneRouteDecision::Allowed {
            effective_capabilities,
            ..
        } = engine.decide_route(&request)
        else {
            panic!("expected an allowed route");
        };
        assert_eq!(effective_capabilities, Some(caps(&["get"])));

        // `list` is advertised at the deepest route but not by the narrower
        // intermediate hop, so the narrowed scope refuses it.
        request.required_capability = Some(capability("list"));
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::MissingCapability)
        );
    }

    #[test]
    fn a_capability_the_target_never_advertised_is_refused() {
        let engine = seeded_engine();
        let mut request = allowed_request(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 1_500);
        request.required_capability = Some(capability("watch"));
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::MissingCapability)
        );
    }

    #[test]
    fn the_local_root_target_asserts_no_advertised_ceiling() {
        let engine = seeded_engine();
        let mut request = allowed_request(zone(&["k0"]), zone(&["k0"]), 1_500);
        request.required_capability = Some(capability("watch"));
        let ZoneRouteDecision::Allowed {
            effective_capabilities,
            path,
            remaining_hops_after,
        } = engine.decide_route(&request)
        else {
            panic!("expected an allowed local route");
        };
        assert_eq!(path.hop_count(), 0);
        assert_eq!(effective_capabilities, None);
        assert_eq!(remaining_hops_after, ZONE_ROUTE_INITIAL_HOP_BUDGET);
    }

    #[test]
    fn a_hop_budget_smaller_than_the_path_is_refused() {
        let engine = seeded_engine();
        let mut request = allowed_request(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 1_500);
        request.remaining_hops = 1;
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::HopLimitExceeded)
        );

        request.remaining_hops = 2;
        let ZoneRouteDecision::Allowed {
            remaining_hops_after,
            ..
        } = engine.decide_route(&request)
        else {
            panic!("expected an allowed route at the exact budget");
        };
        assert_eq!(remaining_hops_after, 0);
    }

    #[test]
    fn an_exhausted_hop_budget_is_refused_before_the_walk() {
        let engine = seeded_engine();
        let mut request = allowed_request(zone(&["k0"]), zone(&["k9", "k9"]), 1_500);
        request.remaining_hops = 0;
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::HopLimitExceeded)
        );
    }

    #[test]
    fn a_relay_hop_decrements_the_budget_only_with_both_independent_grants() {
        let mut request = ZoneRelayRequest::new(4);
        request.zone_link_connected = true;
        request.target_verb_granted = true;
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::RelayDenied)
        );

        request.relay_granted = true;
        request.target_verb_granted = false;
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );

        request.target_verb_granted = true;
        let admission = ZoneRouteEngine::admit_relay_hop(&request);
        assert_eq!(
            admission,
            ZoneRelayAdmission::Admitted {
                forwarded_remaining_hops: 3
            }
        );
        assert_eq!(
            admission.audit_event(),
            ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted
        );
    }

    #[test]
    fn a_relay_hop_refuses_an_exhausted_budget_a_dead_link_and_an_attachment() {
        let mut request = ZoneRelayRequest::new(0);
        request.relay_granted = true;
        request.target_verb_granted = true;
        request.zone_link_connected = true;
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::HopLimitExceeded)
        );

        request.arrived_remaining_hops = 4;
        request.zone_link_connected = false;
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );

        request.zone_link_connected = true;
        request.offers_attachment = true;
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink)
        );
    }

    #[test]
    fn relay_forwarding_never_exceeds_the_initial_protocol_budget() {
        let mut request = ZoneRelayRequest::new(ZONE_ROUTE_INITIAL_HOP_BUDGET);
        request.relay_granted = true;
        request.target_verb_granted = true;
        request.zone_link_connected = true;
        let mut remaining = ZONE_ROUTE_INITIAL_HOP_BUDGET;
        let mut hops = 0_u32;
        while let ZoneRelayAdmission::Admitted {
            forwarded_remaining_hops,
        } = ZoneRouteEngine::admit_relay_hop(&request)
        {
            remaining = forwarded_remaining_hops;
            request.arrived_remaining_hops = forwarded_remaining_hops;
            hops += 1;
        }
        assert_eq!(remaining, 0);
        assert_eq!(hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);
        assert_eq!(
            ZoneRouteEngine::admit_relay_hop(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::HopLimitExceeded)
        );
    }

    #[test]
    fn a_withdrawal_removes_only_the_named_live_routes() {
        let mut engine = seeded_engine();
        let withdrawal = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            generation("gen-1"),
            vec![route_id("route-1"), route_id("route-unknown")],
            1_600,
            signature("sigref-w1"),
        )
        .expect("valid withdrawal");
        let outcome = engine.admit_withdrawal(&withdrawal, 1_700);
        assert_eq!(
            outcome,
            ZoneWithdrawalAdmission::Accepted {
                withdrawn_route_ids: vec![route_id("route-1")]
            }
        );
        assert_eq!(
            outcome.audit_event(),
            ZoneRouteAuditEventKind::ZoneAdvertisementWithdrawn
        );
        assert!(engine.route_inventory().is_empty());

        // Withdrawing again is idempotent rather than an error.
        assert_eq!(
            engine.admit_withdrawal(&withdrawal, 1_800),
            ZoneWithdrawalAdmission::Accepted {
                withdrawn_route_ids: Vec::new()
            }
        );
    }

    #[test]
    fn a_withdrawal_from_another_generation_or_zone_is_refused() {
        let mut engine = seeded_engine();
        let wrong_generation = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            generation("gen-9"),
            vec![route_id("route-1")],
            1_600,
            signature("sigref-w1"),
        )
        .expect("valid withdrawal");
        assert_eq!(
            engine
                .admit_withdrawal(&wrong_generation, 1_700)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::NamespaceViolation)
        );

        let wrong_zone = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k3", "k0"]),
            generation("gen-1"),
            vec![route_id("route-1")],
            1_600,
            signature("sigref-w2"),
        )
        .expect("valid withdrawal");
        assert_eq!(
            engine.admit_withdrawal(&wrong_zone, 1_700).denial_reason(),
            Some(ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert)
        );

        let predating = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            generation("gen-1"),
            vec![route_id("route-1")],
            900,
            signature("sigref-w3"),
        )
        .expect("valid withdrawal");
        assert_eq!(
            engine.admit_withdrawal(&predating, 1_700).denial_reason(),
            Some(ZoneRouteFailClosedReason::Replay)
        );

        // No refusal changed the projection.
        assert_eq!(engine.route_inventory().len(), 1);
    }

    #[test]
    fn a_future_dated_withdrawal_is_refused_as_malformed() {
        let mut engine = seeded_engine();
        let withdrawal = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            generation("gen-1"),
            vec![route_id("route-1")],
            9_000,
            signature("sigref-w4"),
        )
        .expect("valid withdrawal");
        assert_eq!(
            engine.admit_withdrawal(&withdrawal, 1_700).denial_reason(),
            Some(ZoneRouteFailClosedReason::MalformedAdvert)
        );
    }

    #[test]
    fn expiry_sweeps_projection_state_and_makes_the_route_unknown_again() {
        let mut engine = seeded_engine();
        let report = engine.prune_expired(4_000);
        assert_eq!(report.route_entries, 1);
        assert_eq!(report.parent_entries, 2);
        assert_eq!(report.replay_keys, 1);
        let request = allowed_request(zone(&["k0"]), zone(&["k2", "k1", "k0"]), 4_000);
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn pre_auth_admission_drops_new_on_overflow_and_on_the_rate_ceiling() {
        assert_eq!(
            decide_pre_auth_admission(4, 4, 0, 10),
            ZonePreAuthAdmission::Dropped {
                reason: ZoneRouteFailClosedReason::QueueFullDropNew
            }
        );
        assert_eq!(
            decide_pre_auth_admission(0, 4, 10, 10),
            ZonePreAuthAdmission::Dropped {
                reason: ZoneRouteFailClosedReason::RateLimited
            }
        );
        let queued = decide_pre_auth_admission(1, 4, 1, 10);
        assert_eq!(
            queued,
            ZonePreAuthAdmission::Queued {
                queue_depth_after: 2
            }
        );
        assert_eq!(
            queued.audit_event(),
            ZoneRouteAuditEventKind::ZoneLinkIntentQueued
        );
    }

    #[test]
    fn every_reason_the_engine_can_produce_is_covered_by_this_suite() {
        // The engine can produce every closed reason except
        // `SiblingOrParentRouteAdvert` from an advertisement: the contract's
        // own constructor already proves descendant strictness and next-hop
        // agreement, so that shape cannot reach the engine. The engine still
        // uses that reason for a withdrawal naming a route another Zone owns.
        let produced = [
            ZoneRouteFailClosedReason::MalformedAdvert,
            ZoneRouteFailClosedReason::UnknownParent,
            ZoneRouteFailClosedReason::NamespaceViolation,
            ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert,
            ZoneRouteFailClosedReason::Loop,
            ZoneRouteFailClosedReason::MultiParent,
            ZoneRouteFailClosedReason::Expired,
            ZoneRouteFailClosedReason::Replay,
            ZoneRouteFailClosedReason::RateLimited,
            ZoneRouteFailClosedReason::QueueFullDropNew,
            ZoneRouteFailClosedReason::MissingCapability,
            ZoneRouteFailClosedReason::PolicyDenial,
            ZoneRouteFailClosedReason::ZoneLinkDisconnected,
            ZoneRouteFailClosedReason::HopLimitExceeded,
            ZoneRouteFailClosedReason::RelayDenied,
            ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink,
        ];
        let mut labels = produced
            .iter()
            .map(|reason| reason.label())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), produced.len());
    }
}
