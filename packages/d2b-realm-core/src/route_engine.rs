//! Pure tree-route admission and decision engine.
//!
//! The engine consumes already-decoded/verified route metadata and keeps only an
//! in-memory tree view, replay set, bounded audit labels, and low-cardinality
//! telemetry samples. It does not sign, verify signatures, open sockets, relay
//! bytes, or touch live transports.

use crate::capability::{Capability, CapabilitySet};
use crate::frame::OperationKind;
use crate::ids::{ControllerGenerationId, CorrelationId, OperationId, RealmId, RouteId};
use crate::realm::{RealmControllerPlacement, RealmPath};
use crate::routing::{
    DirectShortcutAuthorizationMetadata, DirectShortcutState, DirectShortcutTeardownMetadata,
    DirectShortcutTeardownReason, DiscoveryQueueDropPolicy, DiscoveryQueuePolicy,
    PreAuthAdmissionOutcome, RealmTreeEdge, RouteAdvertisement, RouteAdvertisementEnvelope,
    RouteAuditEventKind, RouteAuditLabels, RouteFailClosedReason, RouteNamespaceAllocation,
    RoutePlacementClass, RoutePolicyRuleId, RouteRealmClass, RouteTelemetryCounterKind,
    RouteTelemetryLabels, RouteTelemetrySample, ShortcutAuthorizationId, TreeRouteDecision,
    TreeRouteDecisionOutcome, TreeRouteHop, TreeRouteHopDirection, TreeRoutePath,
};
use crate::trace_context::TraceContext;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_PARENT_ENTRIES: usize = 4096;
pub const MAX_ROUTE_ENTRIES: usize = 4096;
pub const MAX_REPLAY_KEYS: usize = (MAX_PARENT_ENTRIES + MAX_ROUTE_ENTRIES) * 4;

/// One low-cardinality audit + metric event emitted by the pure route engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEngineEvent {
    pub audit: RouteAuditLabels,
    pub telemetry: RouteTelemetrySample,
}

/// Result of admitting one signed/expiring route advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAdvertisementAdmission {
    pub outcome: RouteAdvertisementAdmissionOutcome,
    pub event: RouteEngineEvent,
}

/// Pure admission outcome for a route advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAdvertisementAdmissionOutcome {
    Accepted { accepted_routes: Vec<RouteId> },
    Denied { reason: RouteFailClosedReason },
}

/// Bounded pre-auth queue/rate-limit helper decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryQueueDecision {
    pub outcome: PreAuthAdmissionOutcome,
    pub queue_depth_after: u32,
    pub event: RouteEngineEvent,
}

/// Snapshot row used for deterministic route-table inspection in tests and
/// future callers. Rows are emitted in `BTreeMap` order by descendant path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInventoryEntry {
    pub descendant: RealmPath,
    pub advertising_realm: RealmPath,
    pub next_hop_child: RealmId,
    pub route_id: RouteId,
    pub capabilities: CapabilitySet,
}

/// Counts returned by a physical expiry sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoutePruneReport {
    pub parent_entries: usize,
    pub route_entries: usize,
    pub replay_keys: usize,
}

/// Request to authorize a direct shortcut over an already-authorized tree path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectShortcutAuthorizationRequest {
    pub shortcut_id: ShortcutAuthorizationId,
    pub decision_id: OperationId,
    pub correlation_id: CorrelationId,
    pub trace: Option<TraceContext>,
    pub source_realm: RealmPath,
    pub target_realm: RealmPath,
    pub operation_kind: OperationKind,
    pub policy_rule_id: Option<RoutePolicyRuleId>,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub allow_direct_shortcut: bool,
}

/// Direct shortcut authorization result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectShortcutAuthorizationDecision {
    Authorized {
        metadata: DirectShortcutAuthorizationMetadata,
        route_decision: TreeRouteDecision,
        event: RouteEngineEvent,
    },
    Denied {
        reason: RouteFailClosedReason,
        route_decision: TreeRouteDecision,
        event: RouteEngineEvent,
    },
}

/// Direct shortcut teardown result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectShortcutTeardownDecision {
    pub metadata: DirectShortcutTeardownMetadata,
    pub event: RouteEngineEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentEntry {
    parent: RealmPath,
    route_id: Option<RouteId>,
    capabilities: CapabilitySet,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteEntry {
    advertising_realm: RealmPath,
    next_hop_child: RealmId,
    route_id: RouteId,
    capabilities: CapabilitySet,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayKey {
    advertising_realm: RealmPath,
    controller_generation: ControllerGenerationId,
    issued_at_unix_seconds: u64,
    signature_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteCapacityLimits {
    max_parent_entries: usize,
    max_route_entries: usize,
    max_replay_keys: usize,
}

impl Default for RouteCapacityLimits {
    fn default() -> Self {
        Self {
            max_parent_entries: MAX_PARENT_ENTRIES,
            max_route_entries: MAX_ROUTE_ENTRIES,
            max_replay_keys: MAX_REPLAY_KEYS,
        }
    }
}

/// Pure in-memory route tree engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTreeEngine {
    local_root: RealmPath,
    placement: RoutePlacementClass,
    parents: BTreeMap<RealmPath, ParentEntry>,
    routes: BTreeMap<RealmPath, RouteEntry>,
    replayed_adverts: BTreeMap<ReplayKey, u64>,
    capacity_limits: RouteCapacityLimits,
}

impl RouteTreeEngine {
    /// Create a route engine rooted at the local realm. No live transport is
    /// created; the engine only tracks tree metadata.
    pub fn new(local_root: RealmPath, placement: RealmControllerPlacement) -> Self {
        Self {
            local_root,
            placement: RoutePlacementClass::from(&placement),
            parents: BTreeMap::new(),
            routes: BTreeMap::new(),
            replayed_adverts: BTreeMap::new(),
            capacity_limits: RouteCapacityLimits::default(),
        }
    }

    /// Create a route engine with explicit in-memory map capacities.
    pub fn with_capacity_limits(
        local_root: RealmPath,
        placement: RealmControllerPlacement,
        max_parent_entries: usize,
        max_route_entries: usize,
        max_replay_keys: usize,
    ) -> Self {
        Self {
            local_root,
            placement: RoutePlacementClass::from(&placement),
            parents: BTreeMap::new(),
            routes: BTreeMap::new(),
            replayed_adverts: BTreeMap::new(),
            capacity_limits: RouteCapacityLimits {
                max_parent_entries,
                max_route_entries,
                max_replay_keys,
            },
        }
    }

    /// Admit one advertisement whose signature/key material was already
    /// verified by the caller. Signature fields are treated as opaque replay
    /// metadata; this function performs no cryptography.
    pub fn admit_advertisement(
        &mut self,
        envelope: &RouteAdvertisementEnvelope,
        allocation: &RouteNamespaceAllocation,
    ) -> RouteAdvertisementAdmission {
        let current_time = envelope.received_at_unix_seconds;
        let advertisement = &envelope.advertisement;
        let denied = |reason| RouteAdvertisementAdmission {
            outcome: RouteAdvertisementAdmissionOutcome::Denied { reason },
            event: route_event(
                RouteAuditEventKind::AdvertisementDenied,
                RouteTelemetryCounterKind::RouteAdvertisementDeniedCount,
                OperationKind::NodeCapabilities,
                Some(reason),
                None,
                self.placement,
                RouteRealmClass::EphemeralDiscovered,
                RouteRealmClass::EphemeralDiscovered,
                1,
            ),
        };

        if advertisement.tree_edge.child != advertisement.advertising_realm
            || advertisement.routes.is_empty()
            || advertisement.expires_at_unix_seconds <= advertisement.issued_at_unix_seconds
            || current_time < advertisement.issued_at_unix_seconds
        {
            return denied(RouteFailClosedReason::MalformedAdvert);
        }
        if current_time >= advertisement.expires_at_unix_seconds {
            return denied(RouteFailClosedReason::Expired);
        }
        if advertisement.tree_edge.parent != self.local_root
            && self
                .parent_entry_at(&advertisement.tree_edge.parent, current_time)
                .is_none()
        {
            return denied(RouteFailClosedReason::UnknownParent);
        }

        if self.replayed_adverts.iter().any(|(key, expires_at)| {
            replay_key_matches(key, envelope) && !is_expired(*expires_at, current_time)
        }) {
            return denied(RouteFailClosedReason::Replay);
        }

        if allocation.tree_edge != advertisement.tree_edge
            || allocation.allocated_to_generation != advertisement.controller_generation
            || advertisement.routes.len() > allocation.max_routes as usize
        {
            return denied(RouteFailClosedReason::NamespaceViolation);
        }

        let mut parent_updates = BTreeMap::new();
        let mut route_updates = BTreeMap::new();
        if let Err(reason) = stage_parent_edge(
            &self.parents,
            &mut parent_updates,
            current_time,
            advertisement.issued_at_unix_seconds,
            advertisement.tree_edge.parent.clone(),
            advertisement.tree_edge.child.clone(),
            None,
            Some(allocation.capability_ceiling.clone()),
            Some(&allocation.capability_ceiling),
            advertisement.expires_at_unix_seconds,
        ) {
            return denied(reason);
        }

        let mut accepted_routes = Vec::new();
        let mut seen_descendants = BTreeSet::new();
        for route in &advertisement.routes {
            if !seen_descendants.insert(&route.descendant) {
                return denied(RouteFailClosedReason::MalformedAdvert);
            }
            if !route
                .descendant
                .is_descendant_of(&advertisement.advertising_realm)
                || direct_child_below(&route.descendant, &advertisement.advertising_realm)
                    .as_ref()
                    .and_then(|child| child.labels().first())
                    != Some(&route.next_hop_child)
            {
                return denied(RouteFailClosedReason::SiblingOrParentRouteAdvert);
            }
            if !prefix_allowed(&route.descendant, &allocation.allowed_prefixes)
                || !route
                    .capabilities
                    .is_subset_of(&allocation.capability_ceiling)
            {
                return denied(RouteFailClosedReason::NamespaceViolation);
            }

            let Some(next_child) =
                direct_child_below(&route.descendant, &advertisement.advertising_realm)
            else {
                return denied(RouteFailClosedReason::MalformedAdvert);
            };
            let (edge_route_id, edge_capabilities) = if next_child == route.descendant {
                (
                    Some(route.route_id.clone()),
                    Some(route.capabilities.clone()),
                )
            } else {
                (None, None)
            };
            if let Err(reason) = stage_parent_edge(
                &self.parents,
                &mut parent_updates,
                current_time,
                advertisement.issued_at_unix_seconds,
                advertisement.advertising_realm.clone(),
                next_child.clone(),
                edge_route_id,
                edge_capabilities,
                None,
                advertisement.expires_at_unix_seconds,
            ) {
                return denied(reason);
            }

            if let Err(reason) = stage_route_entry(
                &self.routes,
                &mut route_updates,
                current_time,
                advertisement.issued_at_unix_seconds,
                advertisement.advertising_realm.clone(),
                route.descendant.clone(),
                route.next_hop_child.clone(),
                route.route_id.clone(),
                route.capabilities.clone(),
                advertisement.expires_at_unix_seconds,
            ) {
                return denied(reason);
            }
            accepted_routes.push(route.route_id.clone());
        }

        let parent_physical_len = projected_physical_len(&self.parents, &parent_updates);
        let route_physical_len = projected_physical_len(&self.routes, &route_updates);
        let replay_physical_len = self.replayed_adverts.len()
            + usize::from(
                !self
                    .replayed_adverts
                    .keys()
                    .any(|key| replay_key_matches(key, envelope)),
            );
        let capacity_pressure = parent_physical_len > self.capacity_limits.max_parent_entries
            || route_physical_len > self.capacity_limits.max_route_entries
            || replay_physical_len > self.capacity_limits.max_replay_keys;

        if capacity_pressure {
            let parent_pruned_len =
                projected_parent_len_after_prune(&self.parents, &parent_updates, current_time);
            let route_pruned_len =
                projected_route_len_after_prune(&self.routes, &route_updates, current_time);
            let replay_pruned_len =
                projected_replay_len_after_prune(&self.replayed_adverts, envelope, current_time);
            if parent_pruned_len > self.capacity_limits.max_parent_entries
                || route_pruned_len > self.capacity_limits.max_route_entries
                || replay_pruned_len > self.capacity_limits.max_replay_keys
            {
                return denied(RouteFailClosedReason::QueueFullDropNew);
            }
        }

        accepted_routes.sort();
        if capacity_pressure {
            self.prune_expired(current_time);
            prune_superseded_replay_keys(&mut self.replayed_adverts, advertisement);
        }
        for (child, entry) in parent_updates {
            self.parents.insert(child, entry);
        }
        for (descendant, entry) in route_updates {
            self.routes.insert(descendant, entry);
        }
        let replay_key = ReplayKey {
            advertising_realm: advertisement.advertising_realm.clone(),
            controller_generation: advertisement.controller_generation.clone(),
            issued_at_unix_seconds: advertisement.issued_at_unix_seconds,
            signature_ref: advertisement.signature.signature_ref.as_str().to_owned(),
        };
        self.replayed_adverts
            .insert(replay_key, advertisement.expires_at_unix_seconds);

        RouteAdvertisementAdmission {
            outcome: RouteAdvertisementAdmissionOutcome::Accepted { accepted_routes },
            event: route_event(
                RouteAuditEventKind::AdvertisementAccepted,
                RouteTelemetryCounterKind::RouteAdvertisementAcceptedCount,
                OperationKind::NodeCapabilities,
                None,
                None,
                self.placement,
                RouteRealmClass::EphemeralDiscovered,
                RouteRealmClass::EphemeralDiscovered,
                1,
            ),
        }
    }

    /// Decide the tree path for one semantic operation.
    pub fn decide_route(
        &self,
        decision_id: OperationId,
        correlation_id: CorrelationId,
        trace: Option<TraceContext>,
        source_realm: RealmPath,
        target_realm: RealmPath,
        operation_kind: OperationKind,
        policy_rule_id: Option<RoutePolicyRuleId>,
    ) -> (TreeRouteDecision, RouteEngineEvent) {
        self.decide_route_at(
            envelope_never_expires_time(),
            decision_id,
            correlation_id,
            trace,
            source_realm,
            target_realm,
            operation_kind,
            policy_rule_id,
        )
    }

    /// Decide the tree path at a verifier-supplied Unix timestamp, ignoring
    /// route entries whose advertisements have expired.
    pub fn decide_route_at(
        &self,
        current_time_unix_seconds: u64,
        decision_id: OperationId,
        correlation_id: CorrelationId,
        trace: Option<TraceContext>,
        source_realm: RealmPath,
        target_realm: RealmPath,
        operation_kind: OperationKind,
        policy_rule_id: Option<RoutePolicyRuleId>,
    ) -> (TreeRouteDecision, RouteEngineEvent) {
        let required_capability = operation_kind.required_capability();
        let result = self.build_path_at(&source_realm, &target_realm, current_time_unix_seconds);
        let outcome = match result {
            Ok(path) => {
                if let Some(capability) = required_capability
                    && !self.target_has_capability_at(
                        &target_realm,
                        capability,
                        current_time_unix_seconds,
                    )
                {
                    TreeRouteDecisionOutcome::Denied {
                        reason: RouteFailClosedReason::MissingCapability,
                    }
                } else {
                    TreeRouteDecisionOutcome::Allowed { path }
                }
            }
            Err(reason) => TreeRouteDecisionOutcome::Denied { reason },
        };

        let reason = match &outcome {
            TreeRouteDecisionOutcome::Allowed { .. } => None,
            TreeRouteDecisionOutcome::Denied { reason } => Some(*reason),
        };
        let event = route_event(
            if reason.is_some() {
                RouteAuditEventKind::RouteDenied
            } else {
                RouteAuditEventKind::RouteAllowed
            },
            if reason.is_some() {
                RouteTelemetryCounterKind::RouteDecisionDeniedCount
            } else {
                RouteTelemetryCounterKind::RouteDecisionAllowedCount
            },
            operation_kind,
            reason,
            policy_rule_id.clone(),
            self.placement,
            self.class_for_at(&source_realm, current_time_unix_seconds),
            self.class_for_at(&target_realm, current_time_unix_seconds),
            1,
        );
        (
            TreeRouteDecision {
                decision_id,
                correlation_id,
                trace,
                source_realm,
                target_realm,
                operation_kind,
                required_capability,
                policy_rule_id,
                outcome,
            },
            event,
        )
    }

    /// Authorize a direct shortcut only when the tree path is already allowed
    /// and policy explicitly permits shortcut metadata for this request.
    pub fn decide_direct_shortcut(
        &self,
        current_time_unix_seconds: u64,
        request: DirectShortcutAuthorizationRequest,
    ) -> DirectShortcutAuthorizationDecision {
        let shortcut_time_reason = if current_time_unix_seconds < request.issued_at_unix_seconds {
            Some(RouteFailClosedReason::PolicyDenial)
        } else if current_time_unix_seconds >= request.expires_at_unix_seconds {
            Some(RouteFailClosedReason::Expired)
        } else {
            None
        };
        let (route_decision, _) = self.decide_route_at(
            current_time_unix_seconds,
            request.decision_id,
            request.correlation_id.clone(),
            request.trace.clone(),
            request.source_realm.clone(),
            request.target_realm.clone(),
            request.operation_kind,
            request.policy_rule_id.clone(),
        );
        let denied = |reason| DirectShortcutAuthorizationDecision::Denied {
            reason,
            route_decision: route_decision.clone(),
            event: route_event(
                RouteAuditEventKind::ShortcutDenied,
                RouteTelemetryCounterKind::ShortcutDeniedCount,
                request.operation_kind,
                Some(reason),
                request.policy_rule_id.clone(),
                self.placement,
                self.class_for_at(&request.source_realm, current_time_unix_seconds),
                self.class_for_at(&request.target_realm, current_time_unix_seconds),
                1,
            ),
        };

        if let Some(reason) = shortcut_time_reason {
            return denied(reason);
        }

        let TreeRouteDecisionOutcome::Allowed { path } = &route_decision.outcome else {
            let TreeRouteDecisionOutcome::Denied { reason } = route_decision.outcome else {
                unreachable!("route decision outcome is exhaustive")
            };
            return denied(reason);
        };
        if !request.allow_direct_shortcut {
            return denied(RouteFailClosedReason::PolicyDenial);
        }

        let Some(metadata) = DirectShortcutAuthorizationMetadata::new(
            request.shortcut_id,
            request.correlation_id,
            request.trace,
            path.nearest_common_ancestor.clone(),
            request.source_realm.clone(),
            request.target_realm.clone(),
            request.operation_kind,
            request.operation_kind.required_capability(),
            path.clone(),
            request.policy_rule_id.clone(),
            DirectShortcutState::Authorized,
            request.issued_at_unix_seconds,
            request.expires_at_unix_seconds,
        ) else {
            return denied(RouteFailClosedReason::PolicyDenial);
        };

        DirectShortcutAuthorizationDecision::Authorized {
            metadata,
            route_decision,
            event: route_event(
                RouteAuditEventKind::ShortcutAuthorized,
                RouteTelemetryCounterKind::ShortcutAuthorizedCount,
                request.operation_kind,
                None,
                request.policy_rule_id,
                self.placement,
                self.class_for_at(&request.source_realm, current_time_unix_seconds),
                self.class_for_at(&request.target_realm, current_time_unix_seconds),
                1,
            ),
        }
    }

    /// Build teardown metadata for an already-authorized shortcut without
    /// touching any live transport.
    pub fn decide_direct_shortcut_teardown(
        &self,
        metadata: &DirectShortcutAuthorizationMetadata,
        reason: DirectShortcutTeardownReason,
        torn_down_at_unix_seconds: u64,
    ) -> DirectShortcutTeardownDecision {
        let teardown = DirectShortcutTeardownMetadata {
            shortcut_id: metadata.shortcut_id.clone(),
            correlation_id: metadata.correlation_id.clone(),
            trace: metadata.trace.clone(),
            source_realm: metadata.source_realm.clone(),
            target_realm: metadata.target_realm.clone(),
            reason,
            torn_down_at_unix_seconds,
        };
        DirectShortcutTeardownDecision {
            metadata: teardown,
            event: route_event(
                RouteAuditEventKind::ShortcutTornDown,
                RouteTelemetryCounterKind::RevocationTeardownCount,
                metadata.operation_kind,
                None,
                metadata.policy_rule_id.clone(),
                self.placement,
                self.class_for_at(&metadata.source_realm, torn_down_at_unix_seconds),
                self.class_for_at(&metadata.target_realm, torn_down_at_unix_seconds),
                1,
            ),
        }
    }

    /// Deterministic route inventory sorted by descendant path.
    pub fn route_inventory(&self) -> Vec<RouteInventoryEntry> {
        self.routes
            .iter()
            .map(|(descendant, route)| RouteInventoryEntry {
                descendant: descendant.clone(),
                advertising_realm: route.advertising_realm.clone(),
                next_hop_child: route.next_hop_child.clone(),
                route_id: route.route_id.clone(),
                capabilities: route.capabilities.clone(),
            })
            .collect()
    }

    /// Physically remove expired parent/route entries and replay keys.
    pub fn prune_expired(&mut self, current_time_unix_seconds: u64) -> RoutePruneReport {
        let parents_before = self.parents.len();
        self.parents.retain(|_, entry| {
            !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
        });

        let routes_before = self.routes.len();
        self.routes.retain(|_, entry| {
            !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
        });

        let replay_before = self.replayed_adverts.len();
        self.replayed_adverts
            .retain(|_, expires_at| !is_expired(*expires_at, current_time_unix_seconds));

        RoutePruneReport {
            parent_entries: parents_before - self.parents.len(),
            route_entries: routes_before - self.routes.len(),
            replay_keys: replay_before - self.replayed_adverts.len(),
        }
    }

    fn build_path_at(
        &self,
        source: &RealmPath,
        target: &RealmPath,
        current_time_unix_seconds: u64,
    ) -> Result<TreeRoutePath, RouteFailClosedReason> {
        if !self.is_known_realm_at(source, current_time_unix_seconds)
            || !self.is_known_realm_at(target, current_time_unix_seconds)
        {
            return Err(RouteFailClosedReason::UnknownParent);
        }
        let Some(ancestor) = nearest_common_ancestor(source, target) else {
            return Err(RouteFailClosedReason::UnknownParent);
        };
        let mut hops = Vec::new();
        let mut current = source.clone();
        let mut seen = BTreeSet::new();
        while current != ancestor {
            if !seen.insert(current.clone()) {
                return Err(RouteFailClosedReason::Loop);
            }
            let Some(entry) = self.parent_entry_at(&current, current_time_unix_seconds) else {
                return Err(RouteFailClosedReason::UnknownParent);
            };
            if seen.contains(&entry.parent) {
                return Err(RouteFailClosedReason::Loop);
            }
            let hop = TreeRouteHop::new(
                current.clone(),
                entry.parent.clone(),
                RealmTreeEdge::new(entry.parent.clone(), current.clone())
                    .ok_or(RouteFailClosedReason::MalformedAdvert)?,
                TreeRouteHopDirection::UpToParent,
                entry.route_id.clone(),
            )
            .ok_or(RouteFailClosedReason::MalformedAdvert)?;
            current = entry.parent.clone();
            hops.push(hop);
        }

        let mut down = Vec::new();
        let mut current = ancestor.clone();
        let branch_labels = &target.labels()[..target.labels().len() - ancestor.labels().len()];
        for label in branch_labels.iter().rev() {
            let child = child_with_label(&current, label.clone())
                .ok_or(RouteFailClosedReason::MalformedAdvert)?;
            let Some(entry) = self.parent_entry_at(&child, current_time_unix_seconds) else {
                return Err(RouteFailClosedReason::UnknownParent);
            };
            if entry.parent != current {
                return Err(RouteFailClosedReason::MultiParent);
            }
            let hop = TreeRouteHop::new(
                current.clone(),
                child.clone(),
                RealmTreeEdge::new(current.clone(), child.clone())
                    .ok_or(RouteFailClosedReason::MalformedAdvert)?,
                TreeRouteHopDirection::DownToChild,
                entry.route_id.clone(),
            )
            .ok_or(RouteFailClosedReason::MalformedAdvert)?;
            current = child;
            down.push(hop);
        }
        hops.extend(down);

        TreeRoutePath::new(source.clone(), target.clone(), ancestor, hops)
            .ok_or(RouteFailClosedReason::MalformedAdvert)
    }

    fn is_known_realm_at(&self, realm: &RealmPath, current_time_unix_seconds: u64) -> bool {
        realm == &self.local_root
            || self
                .parent_entry_at(realm, current_time_unix_seconds)
                .is_some()
            || self
                .route_entry_at(realm, current_time_unix_seconds)
                .is_some()
    }

    fn target_has_capability_at(
        &self,
        target: &RealmPath,
        capability: Capability,
        current_time_unix_seconds: u64,
    ) -> bool {
        if target == &self.local_root {
            return true;
        }
        self.route_entry_at(target, current_time_unix_seconds)
            .map(|route| route.capabilities.has(capability))
            .or_else(|| {
                self.parent_entry_at(target, current_time_unix_seconds)
                    .map(|entry| entry.capabilities.has(capability))
            })
            .unwrap_or(false)
    }

    fn class_for_at(&self, realm: &RealmPath, current_time_unix_seconds: u64) -> RouteRealmClass {
        if realm == &self.local_root {
            RouteRealmClass::LocalRoot
        } else if self.is_known_realm_at(realm, current_time_unix_seconds) {
            RouteRealmClass::EphemeralDiscovered
        } else {
            RouteRealmClass::Unknown
        }
    }

    fn parent_entry_at(
        &self,
        realm: &RealmPath,
        current_time_unix_seconds: u64,
    ) -> Option<&ParentEntry> {
        self.parents
            .get(realm)
            .filter(|entry| !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds))
    }

    fn route_entry_at(
        &self,
        realm: &RealmPath,
        current_time_unix_seconds: u64,
    ) -> Option<&RouteEntry> {
        self.routes
            .get(realm)
            .filter(|entry| !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds))
    }
}

/// Decide queue admission with fail-closed drop-new overflow and bounded
/// per-relay/per-peer rate counters supplied by the caller.
pub fn decide_discovery_queue(
    policy: &DiscoveryQueuePolicy,
    current_depth: u32,
    relay_events_this_minute: u32,
    peer_events_this_minute: u32,
) -> DiscoveryQueueDecision {
    let (outcome, depth, counter, reason) = if current_depth >= policy.max_depth {
        match policy.drop_policy {
            DiscoveryQueueDropPolicy::DropNew => (
                PreAuthAdmissionOutcome::Dropped {
                    reason: RouteFailClosedReason::QueueFullDropNew,
                },
                current_depth,
                RouteTelemetryCounterKind::DiscoveryDropNewCount,
                Some(RouteFailClosedReason::QueueFullDropNew),
            ),
        }
    } else if relay_events_this_minute >= policy.per_relay_rate_limit_per_minute
        || peer_events_this_minute >= policy.per_unverified_peer_rate_limit_per_minute
    {
        (
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::RateLimited,
            },
            current_depth,
            RouteTelemetryCounterKind::PreAuthRateLimitHitCount,
            Some(RouteFailClosedReason::RateLimited),
        )
    } else {
        (
            PreAuthAdmissionOutcome::Queued,
            current_depth + 1,
            RouteTelemetryCounterKind::DiscoveryQueueDepth,
            None,
        )
    };

    DiscoveryQueueDecision {
        outcome,
        queue_depth_after: depth,
        event: route_event(
            if reason.is_some() {
                RouteAuditEventKind::DiscoveryDropped
            } else {
                RouteAuditEventKind::DiscoveryQueued
            },
            counter,
            OperationKind::NodeCapabilities,
            reason,
            None,
            RoutePlacementClass::Unknown,
            RouteRealmClass::Unknown,
            RouteRealmClass::Unknown,
            u64::from(depth),
        ),
    }
}

fn stage_parent_edge(
    parents: &BTreeMap<RealmPath, ParentEntry>,
    parent_updates: &mut BTreeMap<RealmPath, ParentEntry>,
    current_time_unix_seconds: u64,
    issued_at_unix_seconds: u64,
    parent: RealmPath,
    child: RealmPath,
    route_id: Option<RouteId>,
    capabilities: Option<CapabilitySet>,
    capability_ceiling: Option<&CapabilitySet>,
    expires_at_unix_seconds: u64,
) -> Result<(), RouteFailClosedReason> {
    if child == parent
        || would_form_loop_with_updates(
            parents,
            parent_updates,
            &parent,
            &child,
            current_time_unix_seconds,
        )
    {
        return Err(RouteFailClosedReason::Loop);
    }
    if let Some(existing) = parent_updates.get_mut(&child) {
        if existing.parent != parent {
            return Err(RouteFailClosedReason::MultiParent);
        }
        if let Some(candidate) = route_id {
            existing.route_id = Some(candidate);
        }
        if let Some(capabilities) = capabilities {
            existing.capabilities = bound_capabilities(capabilities, capability_ceiling);
        } else if let Some(ceiling) = capability_ceiling {
            existing.capabilities = existing.capabilities.intersection(ceiling);
        }
        existing.expires_at_unix_seconds = expires_at_unix_seconds;
        return Ok(());
    }

    let mut next_route_id = route_id;
    let mut next_capabilities = capabilities;
    if let Some(existing) = parents.get(&child) {
        if !is_expired(existing.expires_at_unix_seconds, current_time_unix_seconds) {
            if issued_at_unix_seconds <= existing.issued_at_unix_seconds {
                return Err(RouteFailClosedReason::Replay);
            }
            if existing.parent != parent {
                return Err(RouteFailClosedReason::MultiParent);
            }
            if next_route_id.is_none() {
                next_route_id = existing.route_id.clone();
            }
            if next_capabilities.is_none() {
                next_capabilities = Some(existing.capabilities.clone());
            }
        }
    }

    parent_updates.insert(
        child,
        ParentEntry {
            parent,
            route_id: next_route_id,
            capabilities: next_capabilities
                .map(|caps| bound_capabilities(caps, capability_ceiling))
                .unwrap_or_default(),
            issued_at_unix_seconds,
            expires_at_unix_seconds,
        },
    );
    Ok(())
}

fn stage_route_entry(
    routes: &BTreeMap<RealmPath, RouteEntry>,
    route_updates: &mut BTreeMap<RealmPath, RouteEntry>,
    current_time_unix_seconds: u64,
    issued_at_unix_seconds: u64,
    advertising_realm: RealmPath,
    descendant: RealmPath,
    next_hop_child: RealmId,
    route_id: RouteId,
    capabilities: CapabilitySet,
    expires_at_unix_seconds: u64,
) -> Result<(), RouteFailClosedReason> {
    if let Some(existing) = route_updates.get(&descendant) {
        if existing.advertising_realm != advertising_realm
            || existing.next_hop_child != next_hop_child
        {
            return Err(RouteFailClosedReason::MultiParent);
        }
        return Ok(());
    }

    if let Some(existing) = routes.get(&descendant) {
        if !is_expired(existing.expires_at_unix_seconds, current_time_unix_seconds) {
            if issued_at_unix_seconds <= existing.issued_at_unix_seconds {
                return Err(RouteFailClosedReason::Replay);
            }
            if existing.advertising_realm != advertising_realm
                || existing.next_hop_child != next_hop_child
            {
                return Err(RouteFailClosedReason::MultiParent);
            }
        }
    }

    route_updates.insert(
        descendant,
        RouteEntry {
            advertising_realm,
            next_hop_child,
            route_id,
            capabilities,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
        },
    );
    Ok(())
}

fn is_expired(expires_at_unix_seconds: u64, current_time_unix_seconds: u64) -> bool {
    current_time_unix_seconds >= expires_at_unix_seconds
}

fn bound_capabilities(
    capabilities: CapabilitySet,
    ceiling: Option<&CapabilitySet>,
) -> CapabilitySet {
    ceiling
        .map(|ceiling| capabilities.intersection(ceiling))
        .unwrap_or(capabilities)
}

fn projected_physical_len<T, U>(
    entries: &BTreeMap<RealmPath, T>,
    updates: &BTreeMap<RealmPath, U>,
) -> usize {
    entries.len()
        + updates
            .keys()
            .filter(|key| !entries.contains_key(*key))
            .count()
}

fn projected_parent_len_after_prune(
    entries: &BTreeMap<RealmPath, ParentEntry>,
    updates: &BTreeMap<RealmPath, ParentEntry>,
    current_time_unix_seconds: u64,
) -> usize {
    let live_entries = entries
        .iter()
        .filter(|(_, entry)| !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds))
        .count();
    let live_updates = updates
        .keys()
        .filter(|key| {
            entries.get(*key).is_some_and(|entry| {
                !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
            })
        })
        .count();
    live_entries + updates.len() - live_updates
}

fn projected_route_len_after_prune(
    entries: &BTreeMap<RealmPath, RouteEntry>,
    updates: &BTreeMap<RealmPath, RouteEntry>,
    current_time_unix_seconds: u64,
) -> usize {
    let live_entries = entries
        .iter()
        .filter(|(_, entry)| !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds))
        .count();
    let live_updates = updates
        .keys()
        .filter(|key| {
            entries.get(*key).is_some_and(|entry| {
                !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds)
            })
        })
        .count();
    live_entries + updates.len() - live_updates
}

fn projected_replay_len_after_prune(
    replayed_adverts: &BTreeMap<ReplayKey, u64>,
    envelope: &RouteAdvertisementEnvelope,
    current_time_unix_seconds: u64,
) -> usize {
    let advertisement = &envelope.advertisement;
    let retained_entries = replayed_adverts
        .iter()
        .filter(|(key, expires_at)| {
            !is_expired(**expires_at, current_time_unix_seconds)
                && !is_superseded_replay_key(key, **expires_at, advertisement)
        })
        .count();
    let replay_key_retained = replayed_adverts.iter().any(|(key, expires_at)| {
        replay_key_matches(key, envelope)
            && !is_expired(*expires_at, current_time_unix_seconds)
            && !is_superseded_replay_key(key, *expires_at, advertisement)
    });
    retained_entries + usize::from(!replay_key_retained)
}

fn replay_key_matches(key: &ReplayKey, envelope: &RouteAdvertisementEnvelope) -> bool {
    let advertisement = &envelope.advertisement;
    key.advertising_realm == advertisement.advertising_realm
        && key.controller_generation == advertisement.controller_generation
        && key.issued_at_unix_seconds == advertisement.issued_at_unix_seconds
        && key.signature_ref == advertisement.signature.signature_ref.as_str()
}

fn is_superseded_replay_key(
    key: &ReplayKey,
    expires_at_unix_seconds: u64,
    advertisement: &RouteAdvertisement,
) -> bool {
    key.advertising_realm == advertisement.advertising_realm
        && key.controller_generation == advertisement.controller_generation
        && key.issued_at_unix_seconds < advertisement.issued_at_unix_seconds
        && expires_at_unix_seconds <= advertisement.expires_at_unix_seconds
}

fn prune_superseded_replay_keys(
    replayed_adverts: &mut BTreeMap<ReplayKey, u64>,
    advertisement: &RouteAdvertisement,
) {
    replayed_adverts
        .retain(|key, expires_at| !is_superseded_replay_key(key, *expires_at, advertisement));
}

fn envelope_never_expires_time() -> u64 {
    0
}

fn parent_entry_with_updates_at<'a>(
    parents: &'a BTreeMap<RealmPath, ParentEntry>,
    parent_updates: &'a BTreeMap<RealmPath, ParentEntry>,
    realm: &RealmPath,
    current_time_unix_seconds: u64,
) -> Option<&'a ParentEntry> {
    if let Some(entry) = parent_updates.get(realm) {
        return Some(entry);
    }
    parents
        .get(realm)
        .filter(|entry| !is_expired(entry.expires_at_unix_seconds, current_time_unix_seconds))
}

fn would_form_loop_with_updates(
    parents: &BTreeMap<RealmPath, ParentEntry>,
    parent_updates: &BTreeMap<RealmPath, ParentEntry>,
    parent: &RealmPath,
    child: &RealmPath,
    current_time_unix_seconds: u64,
) -> bool {
    let mut current = parent;
    let mut seen = BTreeSet::new();
    while let Some(entry) =
        parent_entry_with_updates_at(parents, parent_updates, current, current_time_unix_seconds)
    {
        if current == child || !seen.insert(current.clone()) {
            return true;
        }
        current = &entry.parent;
    }
    current == child
}

fn prefix_allowed(descendant: &RealmPath, prefixes: &[RealmPath]) -> bool {
    prefixes
        .iter()
        .any(|prefix| descendant == prefix || descendant.is_descendant_of(prefix))
}

fn child_with_label(parent: &RealmPath, label: RealmId) -> Option<RealmPath> {
    let mut labels = parent.labels().to_vec();
    labels.insert(0, label);
    RealmPath::new(labels)
}

fn direct_child_below(descendant: &RealmPath, ancestor: &RealmPath) -> Option<RealmPath> {
    let index = descendant
        .labels()
        .len()
        .checked_sub(ancestor.labels().len() + 1)?;
    child_with_label(ancestor, descendant.labels().get(index)?.clone())
}

fn nearest_common_ancestor(left: &RealmPath, right: &RealmPath) -> Option<RealmPath> {
    let common = left
        .labels()
        .iter()
        .rev()
        .zip(right.labels().iter().rev())
        .take_while(|(a, b)| a == b)
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();
    if common.is_empty() {
        return None;
    }
    RealmPath::new(common.into_iter().rev().collect())
}

fn route_event(
    event: RouteAuditEventKind,
    counter: RouteTelemetryCounterKind,
    operation_kind: OperationKind,
    reason: Option<RouteFailClosedReason>,
    policy_rule_id: Option<RoutePolicyRuleId>,
    placement: RoutePlacementClass,
    source_realm_class: RouteRealmClass,
    target_realm_class: RouteRealmClass,
    value: u64,
) -> RouteEngineEvent {
    RouteEngineEvent {
        audit: RouteAuditLabels {
            event,
            source_realm_class,
            target_realm_class,
            placement,
            operation_kind,
            reason,
            policy_rule_id,
        },
        telemetry: RouteTelemetrySample {
            counter,
            labels: RouteTelemetryLabels {
                source_realm_class,
                target_realm_class,
                placement,
                operation_kind: Some(operation_kind),
                reason,
            },
            value,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrollment::{KeyFingerprint, RealmKeyRole};
    use crate::ids::RouteId;
    use crate::routing::{DescendantRoute, DiscoveryQueueDropPolicy, RouteSignature, SignatureRef};
    use crate::token::ProtocolToken;

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(
            labels
                .iter()
                .map(|label| RealmId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn op(id: &str) -> OperationId {
        OperationId::parse(id).unwrap()
    }

    fn corr(id: &str) -> CorrelationId {
        CorrelationId::parse(id).unwrap()
    }

    fn generation_id(id: &str) -> ControllerGenerationId {
        ControllerGenerationId::parse(id).unwrap()
    }

    fn route_id(id: &str) -> RouteId {
        RouteId::parse(id).unwrap()
    }

    fn signature(id: &str) -> RouteSignature {
        RouteSignature {
            algorithm: ProtocolToken::parse("ed25519-v1").unwrap(),
            key_role: RealmKeyRole::ControllerGeneration,
            signing_key_fingerprint: KeyFingerprint::parse(format!("sha256:{}", "c".repeat(64)))
                .unwrap(),
            signature_ref: SignatureRef::parse(id).unwrap(),
        }
    }

    fn allocation(
        parent: RealmPath,
        child: RealmPath,
        generation: &str,
    ) -> RouteNamespaceAllocation {
        RouteNamespaceAllocation::new(
            RealmTreeEdge::new(parent, child.clone()).unwrap(),
            generation_id(generation),
            vec![child],
            16,
            CapabilitySet::empty()
                .with(Capability::Exec)
                .with(Capability::Lifecycle),
        )
        .unwrap()
    }

    fn advert(
        parent: RealmPath,
        child: RealmPath,
        generation: &str,
        sig: &str,
        routes: Vec<DescendantRoute>,
    ) -> RouteAdvertisementEnvelope {
        let advertisement = crate::routing::RouteAdvertisement::new(
            child.clone(),
            RealmTreeEdge::new(parent, child).unwrap(),
            generation_id(generation),
            routes,
            10,
            20,
            signature(sig),
        )
        .unwrap();
        RouteAdvertisementEnvelope {
            admission: crate::routing::UnverifiedPeerAdmissionAttemptMetadata::new(
                op("attempt-1"),
                corr("corr-1"),
                None,
                crate::routing::UnverifiedPeerRef::parse("peer-1").unwrap(),
                crate::routing::DiscoveryIngressClass::ChildRelay,
                None,
                1,
                PreAuthAdmissionOutcome::Queued,
            )
            .unwrap(),
            replay_window: crate::routing::ReplayWindowMetadata::new(
                crate::routing::RouteReplayWindowId::parse("replay-1").unwrap(),
                16,
                1,
                60,
                0,
                1,
            )
            .unwrap(),
            correlation_id: corr("corr-1"),
            trace: None,
            advertisement,
            received_at_unix_seconds: 12,
        }
    }

    fn route(
        descendant: RealmPath,
        next_hop: &str,
        id: &str,
        caps: &[Capability],
    ) -> DescendantRoute {
        DescendantRoute {
            route_id: route_id(id),
            descendant,
            next_hop_child: RealmId::parse(next_hop).unwrap(),
            capabilities: CapabilitySet::from_caps(caps.iter().copied()),
        }
    }

    fn set_advert_times(
        envelope: &mut RouteAdvertisementEnvelope,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        received_at_unix_seconds: u64,
    ) {
        envelope.advertisement.issued_at_unix_seconds = issued_at_unix_seconds;
        envelope.advertisement.expires_at_unix_seconds = expires_at_unix_seconds;
        envelope.received_at_unix_seconds = received_at_unix_seconds;
    }

    fn assert_capacity_denied(admission: &RouteAdvertisementAdmission) {
        assert_eq!(
            admission.outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::QueueFullDropNew
            }
        );
        assert_eq!(
            admission.event.audit.reason,
            Some(RouteFailClosedReason::QueueFullDropNew)
        );
        assert_eq!(
            admission.event.telemetry.labels.reason,
            Some(RouteFailClosedReason::QueueFullDropNew)
        );
    }

    fn assert_advert_denied(
        admission: &RouteAdvertisementAdmission,
        reason: RouteFailClosedReason,
    ) {
        assert_eq!(
            admission.outcome,
            RouteAdvertisementAdmissionOutcome::Denied { reason }
        );
        assert_eq!(
            admission.event.audit.event,
            RouteAuditEventKind::AdvertisementDenied
        );
        assert_eq!(admission.event.audit.reason, Some(reason));
        assert_eq!(
            admission.event.telemetry.counter,
            RouteTelemetryCounterKind::RouteAdvertisementDeniedCount
        );
        assert_eq!(admission.event.telemetry.labels.reason, Some(reason));
    }

    fn engine_with_nested_routes() -> RouteTreeEngine {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let api = realm(&["api", "payments", "work", "local"]);
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::GatewayVm);
        let work_advert = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Lifecycle],
            )],
        );
        assert!(matches!(
            engine
                .admit_advertisement(&work_advert, &allocation(local, work, "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));

        let mut payments_advert = advert(
            realm(&["work", "local"]),
            payments.clone(),
            "gen-payments",
            "sig-payments",
            vec![route(api, "api", "route-api", &[Capability::Exec])],
        );
        set_advert_times(&mut payments_advert, 11, 30, 12);
        assert!(matches!(
            engine
                .admit_advertisement(
                    &payments_advert,
                    &allocation(realm(&["work", "local"]), payments, "gen-payments"),
                )
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        engine
    }

    #[test]
    fn admits_valid_nested_route_and_decides_nca_path() {
        let engine = engine_with_nested_routes();
        let (decision, event) = engine.decide_route(
            op("decision-1"),
            corr("corr-2"),
            None,
            realm(&["local"]),
            realm(&["api", "payments", "work", "local"]),
            OperationKind::ExecStart,
            None,
        );
        let TreeRouteDecisionOutcome::Allowed { path } = decision.outcome else {
            panic!("expected allowed route");
        };
        assert_eq!(path.nearest_common_ancestor, realm(&["local"]));
        assert_eq!(path.hops.len(), 3);
        assert_eq!(
            path.hops
                .iter()
                .map(|hop| hop.direction)
                .collect::<Vec<_>>(),
            vec![
                TreeRouteHopDirection::DownToChild,
                TreeRouteHopDirection::DownToChild,
                TreeRouteHopDirection::DownToChild
            ]
        );
        assert_eq!(event.audit.reason, None);
        assert_eq!(
            event.telemetry.counter,
            RouteTelemetryCounterKind::RouteDecisionAllowedCount
        );
    }

    #[test]
    fn denies_sibling_or_parent_advertisements() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let mut envelope = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                realm(&["payments", "work", "local"]),
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        envelope.advertisement.routes[0].descendant = realm(&["dev", "local"]);
        envelope.advertisement.routes[0].next_hop_child = RealmId::parse("dev").unwrap();

        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        let admission = engine.admit_advertisement(&envelope, &allocation(local, work, "gen-work"));
        assert_eq!(
            admission.outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::SiblingOrParentRouteAdvert
            }
        );
        assert_eq!(
            admission.event.telemetry.counter,
            RouteTelemetryCounterKind::RouteAdvertisementDeniedCount
        );
    }

    #[test]
    fn malformed_advertisements_are_denied_with_audit_and_telemetry() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let alloc = allocation(local.clone(), work.clone(), "gen-work");
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);

        let mut future_issued = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-future",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-future",
                &[Capability::Exec],
            )],
        );
        set_advert_times(&mut future_issued, 100, 120, 99);
        assert_advert_denied(
            &engine.admit_advertisement(&future_issued, &alloc),
            RouteFailClosedReason::MalformedAdvert,
        );

        let mut duplicate_descendant = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-duplicate",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-a",
                &[Capability::Exec],
            )],
        );
        duplicate_descendant.advertisement.routes.push(route(
            payments.clone(),
            "payments",
            "route-payments-b",
            &[Capability::Lifecycle],
        ));
        set_advert_times(&mut duplicate_descendant, 11, 40, 12);
        assert_advert_denied(
            &engine.admit_advertisement(&duplicate_descendant, &alloc),
            RouteFailClosedReason::MalformedAdvert,
        );

        let mut mismatched_advertiser = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-mismatch",
            vec![route(
                payments,
                "payments",
                "route-payments-mismatch",
                &[Capability::Exec],
            )],
        );
        mismatched_advertiser.advertisement.advertising_realm = realm(&["dev", "local"]);
        set_advert_times(&mut mismatched_advertiser, 11, 40, 12);
        assert_advert_denied(
            &engine.admit_advertisement(&mismatched_advertiser, &alloc),
            RouteFailClosedReason::MalformedAdvert,
        );
    }

    #[test]
    fn detects_loop_and_multiparent_state() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let mut looped = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        looped.parents.insert(
            work.clone(),
            ParentEntry {
                parent: payments.clone(),
                route_id: None,
                capabilities: CapabilitySet::empty(),
                issued_at_unix_seconds: 9,
                expires_at_unix_seconds: 100,
            },
        );
        looped.parents.insert(
            payments.clone(),
            ParentEntry {
                parent: work.clone(),
                route_id: None,
                capabilities: CapabilitySet::empty(),
                issued_at_unix_seconds: 9,
                expires_at_unix_seconds: 100,
            },
        );
        let (_, event) = looped.decide_route(
            op("decision-loop"),
            corr("corr-loop"),
            None,
            payments.clone(),
            local.clone(),
            OperationKind::GuestHealth,
            None,
        );
        assert_eq!(event.audit.reason, Some(RouteFailClosedReason::Loop));
        assert_eq!(event.audit.event, RouteAuditEventKind::RouteDenied);
        assert_eq!(
            event.telemetry.counter,
            RouteTelemetryCounterKind::RouteDecisionDeniedCount
        );
        assert_eq!(
            event.telemetry.labels.reason,
            Some(RouteFailClosedReason::Loop)
        );

        let mut multiparent =
            RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        multiparent.parents.insert(
            payments.clone(),
            ParentEntry {
                parent: realm(&["dev", "local"]),
                route_id: None,
                capabilities: CapabilitySet::empty(),
                issued_at_unix_seconds: 9,
                expires_at_unix_seconds: 100,
            },
        );
        let envelope = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let admission =
            multiparent.admit_advertisement(&envelope, &allocation(local, work, "gen-work"));
        assert_eq!(
            admission.outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::MultiParent
            }
        );
    }

    #[test]
    fn rejects_expired_and_replayed_advertisements() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let mut expired = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        expired.received_at_unix_seconds = 20;
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::GatewayVm);
        assert_eq!(
            engine
                .admit_advertisement(
                    &expired,
                    &allocation(local.clone(), work.clone(), "gen-work")
                )
                .outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Expired
            }
        );

        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        assert!(matches!(
            engine
                .admit_advertisement(&valid, &allocation(local.clone(), work.clone(), "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert_eq!(
            engine
                .admit_advertisement(&valid, &allocation(local, work, "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Replay
            }
        );
    }

    #[test]
    fn replay_detection_ignores_unsigned_envelope_correlation() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let alloc = allocation(local.clone(), work.clone(), "gen-work");
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut replay = first.clone();
        replay.correlation_id = corr("corr-retry");
        replay.trace = Some(TraceContext::new("trace-retry", "span-retry").unwrap());
        let mut engine = RouteTreeEngine::new(local, RealmControllerPlacement::GatewayVm);

        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert_eq!(
            engine.admit_advertisement(&replay, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Replay
            }
        );
    }

    #[test]
    fn prunes_replay_keys_after_advertisement_expiry() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::GatewayVm);
        assert!(matches!(
            engine
                .admit_advertisement(&valid, &allocation(local.clone(), work.clone(), "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert_eq!(
            engine
                .admit_advertisement(&valid, &allocation(local.clone(), work.clone(), "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Replay
            }
        );

        let report = engine.prune_expired(20);
        assert_eq!(report.replay_keys, 1);
        assert!(engine.replayed_adverts.is_empty());
    }

    #[test]
    fn expired_routes_are_ignored_and_physically_pruned() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::GatewayVm);
        assert!(matches!(
            engine
                .admit_advertisement(&valid, &allocation(local.clone(), work.clone(), "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert!(matches!(
            engine
                .decide_route_at(
                    19,
                    op("decision-live"),
                    corr("corr-live"),
                    None,
                    local.clone(),
                    payments.clone(),
                    OperationKind::ExecStart,
                    None,
                )
                .0
                .outcome,
            TreeRouteDecisionOutcome::Allowed { .. }
        ));

        let (expired, event) = engine.decide_route_at(
            20,
            op("decision-expired"),
            corr("corr-expired"),
            None,
            local.clone(),
            payments,
            OperationKind::ExecStart,
            None,
        );
        assert_eq!(
            expired.outcome,
            TreeRouteDecisionOutcome::Denied {
                reason: RouteFailClosedReason::UnknownParent
            }
        );
        assert_eq!(
            event.audit.reason,
            Some(RouteFailClosedReason::UnknownParent)
        );
        assert_eq!(engine.parents.len(), 2);
        assert_eq!(engine.routes.len(), 1);
        assert_eq!(engine.replayed_adverts.len(), 1);

        let report = engine.prune_expired(20);
        assert_eq!(
            report,
            RoutePruneReport {
                parent_entries: 2,
                route_entries: 1,
                replay_keys: 1,
            }
        );
        assert!(engine.parents.is_empty());
        assert!(engine.routes.is_empty());
        assert!(engine.replayed_adverts.is_empty());
    }

    #[test]
    fn parent_capacity_denies_new_advertisement_without_mutation() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            1,
            1,
            1,
        );

        let admission = engine.admit_advertisement(&valid, &allocation(local, work, "gen-work"));

        assert_capacity_denied(&admission);
        assert!(engine.parents.is_empty());
        assert!(engine.routes.is_empty());
        assert!(engine.replayed_adverts.is_empty());
    }

    #[test]
    fn route_capacity_denies_new_advertisement_without_mutation() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            2,
            0,
            1,
        );

        let admission = engine.admit_advertisement(&valid, &allocation(local, work, "gen-work"));

        assert_capacity_denied(&admission);
        assert!(engine.parents.is_empty());
        assert!(engine.routes.is_empty());
        assert!(engine.replayed_adverts.is_empty());
    }

    #[test]
    fn replay_capacity_denies_new_advertisement_without_mutation() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let valid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            2,
            1,
            0,
        );

        let admission = engine.admit_advertisement(&valid, &allocation(local, work, "gen-work"));

        assert_capacity_denied(&admission);
        assert!(engine.parents.is_empty());
        assert!(engine.routes.is_empty());
        assert!(engine.replayed_adverts.is_empty());
    }

    #[test]
    fn refresh_of_existing_parent_and_route_entries_does_not_consume_map_capacity() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Lifecycle],
            )],
        );
        let mut second = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work-refresh",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-v2",
                &[Capability::Exec],
            )],
        );
        set_advert_times(&mut second, 11, 40, 12);
        let alloc = allocation(local.clone(), work.clone(), "gen-work");
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            2,
            1,
            2,
        );

        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert!(matches!(
            engine.admit_advertisement(&second, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));

        assert_eq!(engine.parents.len(), 2);
        assert_eq!(engine.routes.len(), 1);
        assert_eq!(engine.replayed_adverts.len(), 2);
        let inventory = engine.route_inventory();
        assert_eq!(inventory[0].route_id, route_id("route-payments-v2"));
        assert!(inventory[0].capabilities.has(Capability::Exec));
        assert!(!inventory[0].capabilities.has(Capability::Lifecycle));
    }

    #[test]
    fn stale_issued_at_advertisement_cannot_downgrade_capabilities() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let mut newer = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-newer",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-newer",
                &[Capability::Exec],
            )],
        );
        set_advert_times(&mut newer, 12, 60, 13);
        let mut stale = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-stale",
            vec![route(
                payments,
                "payments",
                "route-payments-stale",
                &[Capability::Lifecycle],
            )],
        );
        set_advert_times(&mut stale, 11, 70, 14);
        let alloc = allocation(local, work, "gen-work");
        let mut engine =
            RouteTreeEngine::new(realm(&["local"]), RealmControllerPlacement::GatewayVm);

        assert!(matches!(
            engine.admit_advertisement(&newer, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert_eq!(
            engine.admit_advertisement(&stale, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Replay
            }
        );
        let inventory = engine.route_inventory();
        assert_eq!(inventory[0].route_id, route_id("route-payments-newer"));
        assert!(inventory[0].capabilities.has(Capability::Exec));
        assert!(!inventory[0].capabilities.has(Capability::Lifecycle));
    }

    #[test]
    fn equal_issued_at_refresh_is_rejected() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-first",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-first",
                &[Capability::Lifecycle],
            )],
        );
        let mut equal = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-equal",
            vec![route(
                payments,
                "payments",
                "route-payments-equal",
                &[Capability::Exec],
            )],
        );
        equal.advertisement.expires_at_unix_seconds = 40;
        let alloc = allocation(local, work, "gen-work");
        let mut engine =
            RouteTreeEngine::new(realm(&["local"]), RealmControllerPlacement::GatewayVm);

        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert_eq!(
            engine.admit_advertisement(&equal, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::Replay
            }
        );
        assert_eq!(
            engine.route_inventory()[0].route_id,
            route_id("route-payments-first")
        );
    }

    #[test]
    fn expired_pruning_frees_route_capacity_before_admission() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let dev = realm(&["dev", "local"]);
        let api = realm(&["api", "dev", "local"]);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut second = advert(
            local.clone(),
            dev.clone(),
            "gen-dev",
            "sig-dev",
            vec![route(api.clone(), "api", "route-api", &[Capability::Exec])],
        );
        set_advert_times(&mut second, 11, 40, 21);
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            2,
            1,
            1,
        );

        assert!(matches!(
            engine
                .admit_advertisement(&first, &allocation(local.clone(), work, "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert!(matches!(
            engine
                .admit_advertisement(&second, &allocation(local.clone(), dev, "gen-dev"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));

        assert_eq!(engine.parents.len(), 2);
        assert_eq!(engine.routes.len(), 1);
        assert_eq!(engine.replayed_adverts.len(), 1);
        assert_eq!(engine.route_inventory()[0].descendant, api);
    }

    #[test]
    fn refreshed_route_updates_capabilities_and_route_id() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Lifecycle],
            )],
        );
        let mut second = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work-2",
            vec![route(
                payments.clone(),
                "payments",
                "route-payments-v2",
                &[Capability::Exec],
            )],
        );
        set_advert_times(&mut second, 11, 40, 12);

        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::GatewayVm);
        let alloc = allocation(local.clone(), work.clone(), "gen-work");
        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert!(matches!(
            engine
                .decide_route_at(
                    12,
                    op("decision-before-refresh"),
                    corr("corr-before-refresh"),
                    None,
                    local.clone(),
                    payments.clone(),
                    OperationKind::ExecStart,
                    None,
                )
                .0
                .outcome,
            TreeRouteDecisionOutcome::Denied {
                reason: RouteFailClosedReason::MissingCapability
            }
        ));

        assert!(matches!(
            engine.admit_advertisement(&second, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        let inventory = engine.route_inventory();
        assert_eq!(inventory[0].route_id, route_id("route-payments-v2"));
        assert!(inventory[0].capabilities.has(Capability::Exec));
        assert!(!inventory[0].capabilities.has(Capability::Lifecycle));
        let (decision, _) = engine.decide_route_at(
            12,
            op("decision-after-refresh"),
            corr("corr-after-refresh"),
            None,
            local,
            payments,
            OperationKind::ExecStart,
            None,
        );
        let TreeRouteDecisionOutcome::Allowed { path } = decision.outcome else {
            panic!("expected refreshed route to allow exec");
        };
        assert_eq!(
            path.hops.last().unwrap().route_id,
            Some(route_id("route-payments-v2"))
        );
    }

    #[test]
    fn failed_admission_does_not_mutate_existing_state() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work",
            vec![route(
                payments,
                "payments",
                "route-payments",
                &[Capability::Exec],
            )],
        );
        let mut invalid = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-invalid",
            vec![route(
                realm(&["api", "work", "local"]),
                "api",
                "route-api",
                &[Capability::Exec],
            )],
        );
        set_advert_times(&mut invalid, 11, 40, 12);
        invalid.advertisement.routes[0].capabilities = CapabilitySet::empty().with(Capability::Usb);
        let alloc = allocation(local, work, "gen-work");
        let mut engine =
            RouteTreeEngine::new(realm(&["local"]), RealmControllerPlacement::GatewayVm);

        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        let before = engine.clone();
        assert_eq!(
            engine.admit_advertisement(&invalid, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Denied {
                reason: RouteFailClosedReason::NamespaceViolation
            }
        );
        assert_eq!(engine, before);
    }

    #[test]
    fn normal_refresh_sequence_does_not_exhaust_replay_capacity() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let alloc = allocation(local.clone(), work.clone(), "gen-work");
        let mut engine = RouteTreeEngine::with_capacity_limits(
            local.clone(),
            RealmControllerPlacement::GatewayVm,
            2,
            1,
            2,
        );

        for index in 0..8 {
            let mut refresh = advert(
                local.clone(),
                work.clone(),
                "gen-work",
                &format!("sig-refresh-{index}"),
                vec![route(
                    payments.clone(),
                    "payments",
                    &format!("route-payments-{index}"),
                    &[Capability::Exec],
                )],
            );
            let issued_at = 10 + index;
            set_advert_times(&mut refresh, issued_at, 100 + index, issued_at + 1);
            refresh.correlation_id = corr(&format!("corr-refresh-{index}"));

            assert!(matches!(
                engine.admit_advertisement(&refresh, &alloc).outcome,
                RouteAdvertisementAdmissionOutcome::Accepted { .. }
            ));
            assert!(engine.replayed_adverts.len() <= 2);
        }

        assert_eq!(
            engine.route_inventory()[0].route_id,
            route_id("route-payments-7")
        );
    }

    #[test]
    fn local_root_targets_short_circuit_route_capability_lookup() {
        let local = realm(&["local"]);
        let engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        let (decision, event) = engine.decide_route_at(
            100,
            op("decision-local-root"),
            corr("corr-local-root"),
            None,
            local.clone(),
            local,
            OperationKind::ExecStart,
            None,
        );
        let TreeRouteDecisionOutcome::Allowed { path } = decision.outcome else {
            panic!("expected local root target to be routeable without adverts");
        };
        assert!(path.hops.is_empty());
        assert_eq!(event.audit.target_realm_class, RouteRealmClass::LocalRoot);
    }

    #[test]
    fn transit_parent_edges_do_not_inherit_descendant_capabilities() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let api = realm(&["api", "payments", "work", "local"]);
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        let advert = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work-transit",
            vec![route(
                api.clone(),
                "payments",
                "route-api",
                &[Capability::Exec],
            )],
        );
        assert!(matches!(
            engine
                .admit_advertisement(&advert, &allocation(local.clone(), work, "gen-work"))
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));

        let (transit_decision, transit_event) = engine.decide_route_at(
            15,
            op("decision-transit"),
            corr("corr-transit"),
            None,
            local.clone(),
            payments.clone(),
            OperationKind::ExecStart,
            None,
        );
        assert_eq!(
            transit_decision.outcome,
            TreeRouteDecisionOutcome::Denied {
                reason: RouteFailClosedReason::MissingCapability
            }
        );
        assert_eq!(
            transit_event.telemetry.counter,
            RouteTelemetryCounterKind::RouteDecisionDeniedCount
        );
        assert_eq!(
            transit_event.telemetry.labels.reason,
            Some(RouteFailClosedReason::MissingCapability)
        );

        let transit_edge = engine.parents.get(&payments).unwrap();
        assert!(!transit_edge.capabilities.has(Capability::Exec));
        assert_eq!(engine.route_inventory()[0].descendant, api);
        assert!(
            engine.route_inventory()[0]
                .capabilities
                .has(Capability::Exec)
        );
    }

    #[test]
    fn allocation_ceiling_downgrades_existing_direct_edge_capabilities() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let api = realm(&["api", "work", "local"]);
        let mut alloc = allocation(local.clone(), work.clone(), "gen-work");
        alloc.allowed_prefixes = vec![work.clone()];
        alloc.capability_ceiling = CapabilitySet::empty()
            .with(Capability::Exec)
            .with(Capability::Lifecycle);
        let first = advert(
            local.clone(),
            work.clone(),
            "gen-work",
            "sig-work-1",
            vec![route(
                api.clone(),
                "api",
                "route-work-1",
                &[Capability::Exec],
            )],
        );
        let mut engine = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        assert!(matches!(
            engine.admit_advertisement(&first, &alloc).outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        assert!(
            engine
                .parent_entry_at(&work, 12)
                .unwrap()
                .capabilities
                .has(Capability::Exec)
        );

        let mut downgraded_alloc = allocation(local.clone(), work.clone(), "gen-work");
        downgraded_alloc.allowed_prefixes = vec![work.clone()];
        downgraded_alloc.capability_ceiling = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut refresh = advert(
            local,
            work.clone(),
            "gen-work",
            "sig-work-2",
            vec![route(api, "api", "route-work-2", &[Capability::Lifecycle])],
        );
        set_advert_times(&mut refresh, 11, 40, 12);
        assert!(matches!(
            engine
                .admit_advertisement(&refresh, &downgraded_alloc)
                .outcome,
            RouteAdvertisementAdmissionOutcome::Accepted { .. }
        ));
        let direct_edge = engine.parent_entry_at(&work, 12).unwrap();
        assert!(!direct_edge.capabilities.has(Capability::Exec));
        assert!(direct_edge.capabilities.has(Capability::Lifecycle));
    }

    #[test]
    fn queue_full_drops_new_before_rate_checks() {
        let policy =
            DiscoveryQueuePolicy::new(2, 8, 10, 5, DiscoveryQueueDropPolicy::DropNew).unwrap();
        let full = decide_discovery_queue(&policy, 2, 0, 0);
        assert_eq!(full.queue_depth_after, 2);
        assert_eq!(
            full.outcome,
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::QueueFullDropNew
            }
        );
        assert_eq!(
            full.event.audit.event,
            RouteAuditEventKind::DiscoveryDropped
        );
        assert_eq!(
            full.event.audit.reason,
            Some(RouteFailClosedReason::QueueFullDropNew)
        );
        assert_eq!(
            full.event.telemetry.counter,
            RouteTelemetryCounterKind::DiscoveryDropNewCount
        );
        assert_eq!(
            full.event.telemetry.labels.reason,
            Some(RouteFailClosedReason::QueueFullDropNew)
        );
        let limited = decide_discovery_queue(&policy, 1, 10, 0);
        assert_eq!(
            limited.outcome,
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::RateLimited
            }
        );
        assert_eq!(
            limited.event.audit.event,
            RouteAuditEventKind::DiscoveryDropped
        );
        assert_eq!(
            limited.event.telemetry.counter,
            RouteTelemetryCounterKind::PreAuthRateLimitHitCount
        );
        assert_eq!(
            limited.event.telemetry.labels.reason,
            Some(RouteFailClosedReason::RateLimited)
        );
        let queued = decide_discovery_queue(&policy, 1, 9, 4);
        assert_eq!(queued.outcome, PreAuthAdmissionOutcome::Queued);
        assert_eq!(queued.queue_depth_after, 2);
        assert_eq!(
            queued.event.audit.event,
            RouteAuditEventKind::DiscoveryQueued
        );
        assert_eq!(
            queued.event.telemetry.counter,
            RouteTelemetryCounterKind::DiscoveryQueueDepth
        );
    }

    #[test]
    fn discovery_queue_matches_drop_new_policy_on_overflow() {
        let policy =
            DiscoveryQueuePolicy::new(1, 8, 10, 10, DiscoveryQueueDropPolicy::DropNew).unwrap();
        let decision = decide_discovery_queue(&policy, 1, 0, 0);
        assert_eq!(
            decision.outcome,
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::QueueFullDropNew
            }
        );
        assert_eq!(
            decision.event.telemetry.counter,
            RouteTelemetryCounterKind::DiscoveryDropNewCount
        );
        assert_eq!(
            decision.event.audit.event,
            RouteAuditEventKind::DiscoveryDropped
        );
    }

    #[test]
    fn direct_shortcut_allowed_denied_and_teardown_are_pure_metadata() {
        let engine = engine_with_nested_routes();
        let request = DirectShortcutAuthorizationRequest {
            shortcut_id: ShortcutAuthorizationId::parse("shortcut-1").unwrap(),
            decision_id: op("shortcut-decision"),
            correlation_id: corr("corr-shortcut"),
            trace: None,
            source_realm: realm(&["local"]),
            target_realm: realm(&["api", "payments", "work", "local"]),
            operation_kind: OperationKind::ExecStart,
            policy_rule_id: RoutePolicyRuleId::parse("policy-route-exec"),
            issued_at_unix_seconds: 15,
            expires_at_unix_seconds: 60,
            allow_direct_shortcut: true,
        };
        let DirectShortcutAuthorizationDecision::Authorized {
            metadata, event, ..
        } = engine.decide_direct_shortcut(15, request.clone())
        else {
            panic!("expected shortcut authorization");
        };
        assert_eq!(metadata.authorizing_ancestor, realm(&["local"]));
        assert_eq!(
            event.telemetry.counter,
            RouteTelemetryCounterKind::ShortcutAuthorizedCount
        );

        let expired = engine.decide_direct_shortcut(31, request.clone());
        assert!(matches!(
            expired,
            DirectShortcutAuthorizationDecision::Denied {
                reason: RouteFailClosedReason::UnknownParent,
                ..
            }
        ));

        let pre_issued = engine.decide_direct_shortcut(14, request.clone());
        assert!(matches!(
            pre_issued,
            DirectShortcutAuthorizationDecision::Denied {
                reason: RouteFailClosedReason::PolicyDenial,
                ..
            }
        ));

        let shortcut_expired = engine.decide_direct_shortcut(60, request.clone());
        assert!(matches!(
            shortcut_expired,
            DirectShortcutAuthorizationDecision::Denied {
                reason: RouteFailClosedReason::Expired,
                ..
            }
        ));

        let denied = engine.decide_direct_shortcut(
            15,
            DirectShortcutAuthorizationRequest {
                allow_direct_shortcut: false,
                ..request
            },
        );
        assert!(matches!(
            denied,
            DirectShortcutAuthorizationDecision::Denied {
                reason: RouteFailClosedReason::PolicyDenial,
                ..
            }
        ));
        let DirectShortcutAuthorizationDecision::Denied { event, .. } = denied else {
            unreachable!("matched denied above")
        };
        assert_eq!(event.audit.event, RouteAuditEventKind::ShortcutDenied);
        assert_eq!(
            event.telemetry.counter,
            RouteTelemetryCounterKind::ShortcutDeniedCount
        );
        assert_eq!(
            event.telemetry.labels.reason,
            Some(RouteFailClosedReason::PolicyDenial)
        );

        let teardown = engine.decide_direct_shortcut_teardown(
            &metadata,
            DirectShortcutTeardownReason::Expired,
            61,
        );
        assert_eq!(teardown.metadata.shortcut_id, metadata.shortcut_id);
        assert_eq!(
            teardown.event.audit.event,
            RouteAuditEventKind::ShortcutTornDown
        );
        assert_eq!(
            teardown.event.telemetry.counter,
            RouteTelemetryCounterKind::RevocationTeardownCount
        );
    }

    #[test]
    fn route_inventory_order_is_deterministic() {
        let local = realm(&["local"]);
        let work = realm(&["work", "local"]);
        let payments = realm(&["payments", "work", "local"]);
        let api = realm(&["api", "work", "local"]);
        let routes_a = vec![
            route(
                payments.clone(),
                "payments",
                "route-payments",
                &[Capability::Exec],
            ),
            route(api.clone(), "api", "route-api", &[Capability::Lifecycle]),
        ];
        let routes_b = vec![routes_a[1].clone(), routes_a[0].clone()];
        let mut first = RouteTreeEngine::new(local.clone(), RealmControllerPlacement::HostLocal);
        let mut second = first.clone();
        let adv_a = advert(local.clone(), work.clone(), "gen-work", "sig-a", routes_a);
        let adv_b = advert(local.clone(), work.clone(), "gen-work", "sig-b", routes_b);
        let alloc = allocation(local, work, "gen-work");
        let RouteAdvertisementAdmissionOutcome::Accepted {
            accepted_routes: accepted_a,
        } = first.admit_advertisement(&adv_a, &alloc).outcome
        else {
            panic!("expected first admission");
        };
        let RouteAdvertisementAdmissionOutcome::Accepted {
            accepted_routes: accepted_b,
        } = second.admit_advertisement(&adv_b, &alloc).outcome
        else {
            panic!("expected second admission");
        };
        assert_eq!(
            accepted_a.iter().map(RouteId::as_str).collect::<Vec<_>>(),
            vec!["route-api", "route-payments"]
        );
        assert_eq!(accepted_a, accepted_b);
        assert_eq!(first.route_inventory(), second.route_inventory());
        assert_eq!(
            first
                .route_inventory()
                .iter()
                .map(|entry| entry.descendant.target_form())
                .collect::<Vec<_>>(),
            vec!["api.work.local", "payments.work.local"]
        );
    }
}
