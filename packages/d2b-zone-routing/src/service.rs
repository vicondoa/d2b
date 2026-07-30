//! Zone routing service surface (`ADR046-routing-016`).
//!
//! This module is the serviceable face of Zone routing: it adapts the v3
//! baseline `RealmServiceServer` into a [`ZoneServiceServer`] serving
//! [`ZONE_SERVICE_NAME`]. It sits strictly *above* the two modules that do the
//! actual work and reimplements neither:
//!
//! - [`ZoneRouteEngine`] owns the admitted route projection, the route
//!   decision, and per-hop relay admission. The service never inspects or
//!   rebuilds projection state; it passes the engine through.
//! - [`ZoneEntrypointResolver`] owns sealed-topology entrypoint selection and
//!   the hand-off to the engine's decision. Every route answer this service
//!   returns is the resolver's own typed outcome, unchanged.
//!
//! What the service adds is exactly what neither of those can own alone: the
//! frozen wire method inventory, the bounded concurrent-dispatch ceiling, the
//! bounded audit ring, the ZonePath-addressed shortcut table, and the
//! read-only topology projection.
//!
//! # Topology projection
//!
//! The projection starts from the sealed, sorted `{ childZone, parentZone }`
//! compiler input and joins *only* authenticated, admitted route status
//! obtained by asking the resolver about each child. It therefore exposes no
//! ZoneLink resource name, UID, spec, status, Provider ref, fingerprint,
//! transport setting, or handle - the service never receives those and has no
//! field able to carry one. A parent keeps no reciprocal ZoneLink row, so
//! there is no parent-store row to project and no handler that could return
//! one.
//!
//! # Fail-closed posture
//!
//! Every refusal in this file is one closed [`ZoneRouteFailClosedReason`].
//! There is no permissive default: every authorization, connectivity, and
//! authentication input defaults to its refusing value, a method with no
//! landed handler is refused at dispatch admission rather than served, and an
//! over-ceiling dispatch or shortcut table is refused rather than grown.
//! Authorization verdicts are *inputs*, exactly as in the engine; this service
//! mints, holds, and presents no authority, and performs no I/O.
//!
//! Nothing here accepts or returns a uid, gid, host path, socket path, store
//! path, transport endpoint, credential, or key material.
//!
//! # Deliberately absent
//!
//! `zone-bootstrap` and `zone-enroll` are frozen in the method inventory but
//! carry no handler. Their contract - the allocator-issued single-use PSK, the
//! IKpsk2 bootstrap that consumes it, and the follow-on Noise_KK enrollment
//! record - belongs to the Zone session work that has not landed
//! (`d2b_contracts::v3::zone_session` is still empty). Rather than invent a
//! session or credential surface, both methods fail closed at dispatch
//! admission; see [`ZoneServiceMethod::has_handler`].

use std::collections::{BTreeSet, VecDeque};

use d2b_contracts::v3::execution_policy::PrimitiveSpecError;
use d2b_contracts::v3::zone_routing::{
    MAX_ZONE_PARENT_ENTRIES, ZONE_ROUTE_INITIAL_HOP_BUDGET, ZonePath, ZoneRouteAuditEventKind,
    ZoneRouteFailClosedReason, ZoneTreeEdge,
};

use crate::engine::{ZoneRelayAdmission, ZoneRelayRequest, ZoneRouteEngine};
use crate::resolver::{
    SealedZoneTopology, ZoneEntrypointRequest, ZoneEntrypointResolution, ZoneEntrypointResolver,
};

/// The frozen v3 Zone service wire name.
///
/// The v2 realm name is an ADR45 wire identifier and is not reused; v3 service
/// identifiers freeze independently.
pub const ZONE_SERVICE_NAME: &str = "d2b.zone.v3.ZoneService";

/// Maximum requests this service dispatches concurrently.
///
/// Preserved from the v3 baseline drive loop.
pub const MAX_DISPATCH_IN_FLIGHT: u32 = 64;

/// Seconds the drive loop waits for in-flight work to drain on shutdown.
///
/// Preserved from the v3 baseline drive loop. The value is a bound the runtime
/// owner applies; this module performs no I/O and does not itself wait.
pub const SHUTDOWN_TIMEOUT_SECONDS: u64 = 5;

/// Default ceiling on live ZonePath-addressed shortcuts.
pub const DEFAULT_MAX_SHORTCUTS: usize = 256;

/// Default capacity of the bounded service audit ring.
pub const DEFAULT_AUDIT_CAPACITY: usize = 1024;

/// Largest value an operator-configured service bound may take.
pub const MAX_CONFIGURED_BOUND: usize = 4096;

/// Render a type's `Debug` as its bare type name.
///
/// Every public type in this module that can hold a [`ZonePath`] opts out of a
/// derived `Debug` so a Zone path can never reach a log, span, or metric
/// through an incidental format of a container that holds one. The macro is
/// module-private and adds no public item.
macro_rules! redacted_service_debug {
    ($type_name:ident) => {
        impl ::core::fmt::Debug for $type_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(concat!(stringify!($type_name), "(redacted)"))
            }
        }
    };
}

/// The closed set of `d2b.zone.v3.ZoneService` methods.
///
/// The inventory is frozen here in full, including the two methods whose
/// handler has not landed, so the wire surface does not shift when they do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneServiceMethod {
    /// One-time IKpsk2 bootstrap consuming the allocator-issued single-use
    /// PSK. No handler has landed.
    ZoneBootstrap,
    /// Noise_KK enrollment following a consumed bootstrap. No handler has
    /// landed.
    ZoneEnroll,
    /// Resolve a target Zone to its sealed entrypoint and decide the route.
    ResolveZoneRoute,
    /// Authorize a ZonePath-addressed shortcut to a resolved entrypoint.
    AuthorizeZoneShortcut,
    /// Revoke a previously authorized shortcut.
    RevokeZoneShortcut,
    /// Record that a shortcut was torn down by its user.
    ReportZoneShortcutClose,
    /// Project one sealed topology row plus its authenticated route status.
    ZoneInspect,
    /// Project every sealed topology row plus its authenticated route status.
    ZoneTopologyList,
    /// Report the topology projection only when it changed.
    ZoneTopologyWatch,
    /// Forward one already-admitted call across one authorized hop.
    ZoneRelayHop,
}

impl ZoneServiceMethod {
    /// The stable kebab-case wire method name.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::ZoneBootstrap => "zone-bootstrap",
            Self::ZoneEnroll => "zone-enroll",
            Self::ResolveZoneRoute => "resolve-zone-route",
            Self::AuthorizeZoneShortcut => "authorize-zone-shortcut",
            Self::RevokeZoneShortcut => "revoke-zone-shortcut",
            Self::ReportZoneShortcutClose => "report-zone-shortcut-close",
            Self::ZoneInspect => "zone-inspect",
            Self::ZoneTopologyList => "zone-topology-list",
            Self::ZoneTopologyWatch => "zone-topology-watch",
            Self::ZoneRelayHop => "zone-relay-hop",
        }
    }

    /// Whether a handler for this method has landed.
    ///
    /// A method without a handler is refused at dispatch admission with
    /// [`ZoneRouteFailClosedReason::PolicyDenial`]; it is never served by a
    /// stand-in. The Zone session contract that `zone-bootstrap` and
    /// `zone-enroll` require has not landed, so guessing at it here would be
    /// worse than refusing.
    pub const fn has_handler(self) -> bool {
        !matches!(self, Self::ZoneBootstrap | Self::ZoneEnroll)
    }
}

/// Operator-configurable service bounds.
///
/// Defaults are preserved from the v3 baseline service limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneServiceLimits {
    /// Live ZonePath-addressed shortcuts this service will hold.
    pub max_shortcuts: usize,
    /// Entries the bounded audit ring retains.
    pub audit_capacity: usize,
    /// Requests dispatched concurrently.
    pub max_dispatch_in_flight: u32,
}

impl Default for ZoneServiceLimits {
    fn default() -> Self {
        Self {
            max_shortcuts: DEFAULT_MAX_SHORTCUTS,
            audit_capacity: DEFAULT_AUDIT_CAPACITY,
            max_dispatch_in_flight: MAX_DISPATCH_IN_FLIGHT,
        }
    }
}

impl ZoneServiceLimits {
    /// Build bounds, refusing a zero or over-ceiling value.
    ///
    /// A zero bound would make the service unable to serve anything, and an
    /// over-ceiling bound would let configuration grow memory without limit;
    /// both fail closed with [`PrimitiveSpecError::TooManyEntries`] and
    /// [`PrimitiveSpecError::MissingRequiredField`] respectively.
    pub const fn new(
        max_shortcuts: usize,
        audit_capacity: usize,
        max_dispatch_in_flight: u32,
    ) -> Result<Self, PrimitiveSpecError> {
        if max_shortcuts == 0 || audit_capacity == 0 || max_dispatch_in_flight == 0 {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if max_shortcuts > MAX_CONFIGURED_BOUND
            || audit_capacity > MAX_CONFIGURED_BOUND
            || max_dispatch_in_flight > MAX_DISPATCH_IN_FLIGHT
        {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            max_shortcuts,
            audit_capacity,
            max_dispatch_in_flight,
        })
    }
}

/// One bounded audit record.
///
/// The record carries only closed enumerations: the method, the event kind,
/// and the refusal reason when there is one. It holds no Zone path, capability,
/// route identifier, peer identity, or payload, so it is safe to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneServiceAuditEvent {
    /// The method that produced the record.
    pub method: ZoneServiceMethod,
    /// The audit event kind.
    pub kind: ZoneRouteAuditEventKind,
    /// The closed refusal reason, when the outcome was a refusal.
    pub denial_reason: Option<ZoneRouteFailClosedReason>,
}

/// The outcome of admitting one request into the dispatch window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneDispatchAdmission {
    /// The request was admitted.
    Admitted {
        /// In-flight requests after this admission, including it.
        in_flight_after: u32,
    },
    /// The request was refused.
    Refused {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneDispatchAdmission {
    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Admitted { .. } => None,
            Self::Refused { reason } => Some(*reason),
        }
    }
}

/// The inputs one topology projection is evaluated against.
///
/// Every input defaults to its refusing value in
/// [`ZoneTopologyRequest::new`], matching [`ZoneEntrypointRequest`], so a
/// caller that forgets one gets `Unreachable` rows rather than an
/// optimistically reachable projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneTopologyRequest {
    /// The caller-supplied projection time in Unix seconds.
    pub current_time_unix_seconds: u64,
    /// Whether the caller's authorizer allowed the read.
    pub policy_allows: bool,
    /// Whether the uplink is established.
    pub zone_link_connected: bool,
    /// Whether the backing route projection was admitted from an
    /// authenticated advertisement.
    pub route_projection_authenticated: bool,
}

impl ZoneTopologyRequest {
    /// A projection request with refusing defaults for every input.
    pub const fn new(current_time_unix_seconds: u64) -> Self {
        Self {
            current_time_unix_seconds,
            policy_allows: false,
            zone_link_connected: false,
            route_projection_authenticated: false,
        }
    }
}

/// The joined route status of one sealed topology row.
#[derive(Clone, PartialEq, Eq)]
pub enum ZoneTopologyStatus {
    /// The child is reachable through an authenticated, admitted, unexpired
    /// route projection.
    Reachable,
    /// The child is not reachable, for one closed reason. A withdrawn or
    /// expired projection and an absent one are indistinguishable here by
    /// design: all report [`ZoneRouteFailClosedReason::UnknownParent`], so the
    /// projection never discloses that a route once existed.
    Unreachable {
        /// The closed reason.
        reason: ZoneRouteFailClosedReason,
    },
}

redacted_service_debug!(ZoneTopologyStatus);

/// One projected topology row.
///
/// The row is exactly the sealed compiler input pair plus joined route status.
/// There is no field for, and no way to attach, a ZoneLink resource name, UID,
/// spec, status, Provider ref, fingerprint, transport setting, or handle.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneTopologyRow {
    /// The sealed child Zone.
    pub child_zone: ZonePath,
    /// The sealed parent Zone.
    pub parent_zone: ZonePath,
    /// The joined route status.
    pub status: ZoneTopologyStatus,
}

redacted_service_debug!(ZoneTopologyRow);

/// One topology-watch report.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneTopologyWatchUpdate {
    /// The monotonic projection revision this report carries.
    pub revision: u64,
    /// The full projection at that revision, ordered by child Zone.
    pub rows: Vec<ZoneTopologyRow>,
}

redacted_service_debug!(ZoneTopologyWatchUpdate);

/// The outcome of one shortcut mutation.
#[derive(Clone, PartialEq, Eq)]
pub enum ZoneShortcutOutcome {
    /// A shortcut to this entrypoint is now live.
    Authorized {
        /// The sealed entrypoint Zone the shortcut addresses. Shortcuts are
        /// addressed by Zone path; no handle is minted.
        entrypoint_zone: ZonePath,
        /// Hops left after paying for the resolved path.
        remaining_hops_after: u32,
    },
    /// The addressed shortcut is no longer live.
    Closed,
    /// The mutation was refused.
    Refused {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneShortcutOutcome {
    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Authorized { .. } | Self::Closed => None,
            Self::Refused { reason } => Some(*reason),
        }
    }
}

redacted_service_debug!(ZoneShortcutOutcome);

/// The `d2b.zone.v3.ZoneService` handler for one Zone.
///
/// The Zone runtime instantiates exactly one of these per Zone. It composes
/// [`ZoneEntrypointResolver`] over the sealed topology with a caller-supplied
/// [`ZoneRouteEngine`]; the engine is passed to each call rather than owned, so
/// the service never becomes a second home for projection state.
pub struct ZoneServiceServer {
    resolver: ZoneEntrypointResolver,
    /// The sealed `{ childZone, parentZone }` rows, sorted and deduplicated.
    rows: Vec<(ZonePath, ZonePath)>,
    limits: ZoneServiceLimits,
    in_flight: u32,
    shortcuts: BTreeSet<ZonePath>,
    audit: VecDeque<ZoneServiceAuditEvent>,
    watch_revision: u64,
    last_projection: Option<Vec<ZoneTopologyRow>>,
}

impl ZoneServiceServer {
    /// Build a server over the sealed compiler topology with default bounds.
    pub fn new(local_root: ZonePath, edges: Vec<ZoneTreeEdge>) -> Result<Self, PrimitiveSpecError> {
        Self::with_limits(local_root, edges, ZoneServiceLimits::default())
    }

    /// Build a server over the sealed compiler topology with explicit bounds.
    ///
    /// The edges are sealed by [`SealedZoneTopology::seal`], which is the sole
    /// validator of the topology's shape; this constructor adds only the
    /// row-count ceiling and the sorted, deduplicated projection input.
    pub fn with_limits(
        local_root: ZonePath,
        edges: Vec<ZoneTreeEdge>,
        limits: ZoneServiceLimits,
    ) -> Result<Self, PrimitiveSpecError> {
        if edges.len() > MAX_ZONE_PARENT_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut rows: Vec<(ZonePath, ZonePath)> = edges
            .iter()
            .map(|edge| (edge.child().clone(), edge.parent().clone()))
            .collect();
        rows.sort();
        rows.dedup();

        let topology = SealedZoneTopology::seal(local_root, edges)?;
        Ok(Self {
            resolver: ZoneEntrypointResolver::new(topology),
            rows,
            limits,
            in_flight: 0,
            shortcuts: BTreeSet::new(),
            audit: VecDeque::new(),
            watch_revision: 0,
            last_projection: None,
        })
    }

    /// Borrow the entrypoint resolver this service composes.
    pub const fn resolver(&self) -> &ZoneEntrypointResolver {
        &self.resolver
    }

    /// Requests currently in flight.
    pub const fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// The bounded audit ring, oldest first.
    pub fn audit_events(&self) -> impl ExactSizeIterator<Item = &ZoneServiceAuditEvent> {
        self.audit.iter()
    }

    /// Admit one request into the dispatch window.
    ///
    /// Refuses a method with no landed handler with
    /// [`ZoneRouteFailClosedReason::PolicyDenial`], and a request beyond the
    /// concurrency ceiling with
    /// [`ZoneRouteFailClosedReason::QueueFullDropNew`]. Overflow drops the new
    /// request rather than displacing in-flight work, matching the engine's
    /// pre-authentication admission.
    pub fn begin_dispatch(&mut self, method: ZoneServiceMethod) -> ZoneDispatchAdmission {
        if !method.has_handler() {
            let reason = ZoneRouteFailClosedReason::PolicyDenial;
            self.record(
                method,
                ZoneRouteAuditEventKind::ZoneRouteDenied,
                Some(reason),
            );
            return ZoneDispatchAdmission::Refused { reason };
        }
        if self.in_flight >= self.limits.max_dispatch_in_flight {
            let reason = ZoneRouteFailClosedReason::QueueFullDropNew;
            self.record(
                method,
                ZoneRouteAuditEventKind::ZoneRouteDenied,
                Some(reason),
            );
            return ZoneDispatchAdmission::Refused { reason };
        }
        self.in_flight += 1;
        ZoneDispatchAdmission::Admitted {
            in_flight_after: self.in_flight,
        }
    }

    /// Release one admitted request from the dispatch window.
    ///
    /// Saturating, so an unbalanced release can never wrap the counter into a
    /// window far larger than the ceiling.
    pub fn end_dispatch(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    /// Serve `resolve-zone-route`.
    ///
    /// The answer is the resolver's own outcome, unchanged; the service adds
    /// only the audit record.
    pub fn resolve_zone_route(
        &mut self,
        engine: &ZoneRouteEngine,
        request: &ZoneEntrypointRequest,
    ) -> ZoneEntrypointResolution {
        let resolution = self.resolver.resolve(engine, request);
        self.record(
            ZoneServiceMethod::ResolveZoneRoute,
            resolution.audit_event(),
            resolution.denial_reason(),
        );
        resolution
    }

    /// Serve `zone-relay-hop`.
    ///
    /// Forwarding admission is the engine's, which requires the canonical
    /// `relay` grant and the target verb grant independently at every hop; the
    /// service adds only the audit record.
    pub fn admit_relay_hop(&mut self, request: &ZoneRelayRequest) -> ZoneRelayAdmission {
        let admission = ZoneRouteEngine::admit_relay_hop(request);
        self.record(
            ZoneServiceMethod::ZoneRelayHop,
            admission.audit_event(),
            admission.denial_reason(),
        );
        admission
    }

    /// Serve `zone-topology-list`.
    ///
    /// Rows are the sealed compiler input, ordered by child Zone, joined with
    /// the authenticated admitted route status of each child.
    pub fn list_topology(
        &self,
        engine: &ZoneRouteEngine,
        request: &ZoneTopologyRequest,
    ) -> Vec<ZoneTopologyRow> {
        self.rows
            .iter()
            .map(|(child, parent)| self.project_row(engine, request, child, parent))
            .collect()
    }

    /// Serve `zone-inspect` for one Zone.
    ///
    /// Returns `None` for a Zone with no sealed row. The local root has no
    /// parent and therefore no row, and a parent keeps no reciprocal ZoneLink,
    /// so there is no parent-store row for this handler to return.
    pub fn inspect_zone(
        &self,
        engine: &ZoneRouteEngine,
        child_zone: &ZonePath,
        request: &ZoneTopologyRequest,
    ) -> Option<ZoneTopologyRow> {
        self.rows
            .iter()
            .find(|(child, _)| child == child_zone)
            .map(|(child, parent)| self.project_row(engine, request, child, parent))
    }

    /// Serve `zone-topology-watch`.
    ///
    /// Reports `Some` only when the projection differs from the last reported
    /// one, with a monotonically increasing revision. The first poll always
    /// reports. This is change detection over the same read-only projection;
    /// it opens no stream and holds no peer state.
    pub fn poll_topology_watch(
        &mut self,
        engine: &ZoneRouteEngine,
        request: &ZoneTopologyRequest,
    ) -> Option<ZoneTopologyWatchUpdate> {
        let rows = self.list_topology(engine, request);
        if self.last_projection.as_ref() == Some(&rows) {
            return None;
        }
        self.watch_revision += 1;
        self.last_projection = Some(rows.clone());
        Some(ZoneTopologyWatchUpdate {
            revision: self.watch_revision,
            rows,
        })
    }

    /// Serve `authorize-zone-shortcut`.
    ///
    /// A shortcut is authorized only for an entrypoint the resolver resolves
    /// for this very request, so a shortcut can never widen what an ordinary
    /// route decision would allow. The shortcut is addressed by the resolved
    /// entrypoint Zone path; no handle, session, or credential is minted.
    pub fn authorize_zone_shortcut(
        &mut self,
        engine: &ZoneRouteEngine,
        request: &ZoneEntrypointRequest,
    ) -> ZoneShortcutOutcome {
        let method = ZoneServiceMethod::AuthorizeZoneShortcut;
        let outcome = match self.resolver.resolve(engine, request) {
            ZoneEntrypointResolution::Refused { reason } => ZoneShortcutOutcome::Refused { reason },
            ZoneEntrypointResolution::Resolved {
                entrypoint_zone,
                remaining_hops_after,
                ..
            } => {
                if !self.shortcuts.contains(&entrypoint_zone)
                    && self.shortcuts.len() >= self.limits.max_shortcuts
                {
                    ZoneShortcutOutcome::Refused {
                        reason: ZoneRouteFailClosedReason::QueueFullDropNew,
                    }
                } else {
                    self.shortcuts.insert(entrypoint_zone.clone());
                    ZoneShortcutOutcome::Authorized {
                        entrypoint_zone,
                        remaining_hops_after,
                    }
                }
            }
        };
        let kind = match outcome {
            ZoneShortcutOutcome::Authorized { .. } => {
                ZoneRouteAuditEventKind::ZoneLinkShortcutAuthorized
            }
            _ => ZoneRouteAuditEventKind::ZoneRouteDenied,
        };
        self.record(method, kind, outcome.denial_reason());
        outcome
    }

    /// Serve `revoke-zone-shortcut`.
    ///
    /// Revoking a shortcut that is not live is refused with
    /// [`ZoneRouteFailClosedReason::UnknownParent`] rather than reported as a
    /// success, so a caller cannot probe which entrypoints are live by
    /// distinguishing a no-op from a removal.
    pub fn revoke_zone_shortcut(&mut self, entrypoint_zone: &ZonePath) -> ZoneShortcutOutcome {
        self.close_shortcut(
            ZoneServiceMethod::RevokeZoneShortcut,
            ZoneRouteAuditEventKind::ZoneLinkRevoked,
            entrypoint_zone,
        )
    }

    /// Serve `report-zone-shortcut-close`.
    ///
    /// Identical bookkeeping to revocation, distinguished only by the audit
    /// event kind: the user tore the shortcut down rather than policy revoking
    /// it.
    pub fn report_zone_shortcut_close(
        &mut self,
        entrypoint_zone: &ZonePath,
    ) -> ZoneShortcutOutcome {
        self.close_shortcut(
            ZoneServiceMethod::ReportZoneShortcutClose,
            ZoneRouteAuditEventKind::ZoneLinkShortcutTornDown,
            entrypoint_zone,
        )
    }

    /// Whether a shortcut to this entrypoint is live.
    pub fn shortcut_is_live(&self, entrypoint_zone: &ZonePath) -> bool {
        self.shortcuts.contains(entrypoint_zone)
    }

    fn close_shortcut(
        &mut self,
        method: ZoneServiceMethod,
        closed_kind: ZoneRouteAuditEventKind,
        entrypoint_zone: &ZonePath,
    ) -> ZoneShortcutOutcome {
        if self.shortcuts.remove(entrypoint_zone) {
            self.record(method, closed_kind, None);
            return ZoneShortcutOutcome::Closed;
        }
        let reason = ZoneRouteFailClosedReason::UnknownParent;
        self.record(
            method,
            ZoneRouteAuditEventKind::ZoneRouteDenied,
            Some(reason),
        );
        ZoneShortcutOutcome::Refused { reason }
    }

    fn project_row(
        &self,
        engine: &ZoneRouteEngine,
        request: &ZoneTopologyRequest,
        child: &ZonePath,
        parent: &ZonePath,
    ) -> ZoneTopologyRow {
        let mut entrypoint =
            ZoneEntrypointRequest::new(child.clone(), request.current_time_unix_seconds);
        entrypoint.policy_allows = request.policy_allows;
        entrypoint.zone_link_connected = request.zone_link_connected;
        entrypoint.route_projection_authenticated = request.route_projection_authenticated;
        entrypoint.remaining_hops = ZONE_ROUTE_INITIAL_HOP_BUDGET;

        let status = match self.resolver.resolve(engine, &entrypoint) {
            ZoneEntrypointResolution::Resolved { .. } => ZoneTopologyStatus::Reachable,
            ZoneEntrypointResolution::Refused { reason } => {
                ZoneTopologyStatus::Unreachable { reason }
            }
        };
        ZoneTopologyRow {
            child_zone: child.clone(),
            parent_zone: parent.clone(),
            status,
        }
    }

    fn record(
        &mut self,
        method: ZoneServiceMethod,
        kind: ZoneRouteAuditEventKind,
        denial_reason: Option<ZoneRouteFailClosedReason>,
    ) {
        if self.audit.len() >= self.limits.audit_capacity {
            self.audit.pop_front();
        }
        self.audit.push_back(ZoneServiceAuditEvent {
            method,
            kind,
            denial_reason,
        });
    }
}

redacted_service_debug!(ZoneServiceServer);

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::{
        ZONE_ROUTING_SCHEMA_VERSION, ZoneDescendantRoute, ZoneLabelId,
        ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation, ZoneLinkRouteAdvertisement,
        ZoneRouteCapability, ZoneRouteCapabilitySet, ZoneRouteId, ZoneRouteKeyRole,
        ZoneRouteSignature, ZoneRouteSignatureAlgorithm, ZoneRouteSignatureRef,
        ZoneSigningKeyFingerprint,
    };

    use crate::engine::ZoneAdvertisementAdmission;

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

    fn edge(parent: &[&str], child: &[&str]) -> ZoneTreeEdge {
        ZoneTreeEdge::new(zone(parent), zone(child)).expect("direct child edge")
    }

    fn edges() -> Vec<ZoneTreeEdge> {
        vec![
            edge(&["k0"], &["k1", "k0"]),
            edge(&["k1", "k0"], &["k2", "k1", "k0"]),
        ]
    }

    fn server() -> ZoneServiceServer {
        ZoneServiceServer::new(zone(&["k0"]), edges()).expect("well formed topology")
    }

    /// Engine rooted at k0 with an admitted, authenticated route to k2.
    fn seeded_engine() -> ZoneRouteEngine {
        let mut engine = ZoneRouteEngine::new(zone(&["k0"]));
        let advertisement = ZoneLinkRouteAdvertisement::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            edge(&["k0"], &["k1", "k0"]),
            ZoneLinkControllerGeneration::parse("gen-1").expect("valid generation"),
            vec![ZoneDescendantRoute::new(
                ZoneRouteId::parse("route-1").expect("valid route id"),
                zone(&["k2", "k1", "k0"]),
                ZoneLabelId::parse("k2").expect("valid label"),
                caps(&["get", "list"]),
            )],
            1_000,
            4_000,
            ZoneRouteSignature::new(
                ZoneRouteSignatureAlgorithm::Ed25519Blake3,
                ZoneRouteKeyRole::ZoneControllerRouting,
                ZoneSigningKeyFingerprint::parse(format!("sha256.{}", "b".repeat(64)))
                    .expect("valid fingerprint"),
                ZoneRouteSignatureRef::parse("sigref-1").expect("valid signature ref"),
            ),
        )
        .expect("valid advertisement");
        let allocation = ZoneLinkNamespaceAllocation::new(
            edge(&["k0"], &["k1", "k0"]),
            ZoneLinkControllerGeneration::parse("gen-1").expect("valid generation"),
            vec![zone(&["k1", "k0"])],
            8,
            caps(&["get", "list", "watch"]),
        )
        .expect("valid allocation");
        assert!(matches!(
            engine.admit_advertisement(&advertisement, &allocation, 1_500),
            ZoneAdvertisementAdmission::Accepted { .. }
        ));
        engine
    }

    fn allowed_topology_request() -> ZoneTopologyRequest {
        let mut request = ZoneTopologyRequest::new(1_500);
        request.policy_allows = true;
        request.zone_link_connected = true;
        request.route_projection_authenticated = true;
        request
    }

    fn allowed_entrypoint_request(target: ZonePath) -> ZoneEntrypointRequest {
        let mut request = ZoneEntrypointRequest::new(target, 1_500);
        request.policy_allows = true;
        request.zone_link_connected = true;
        request.route_projection_authenticated = true;
        request
    }

    // -- wire inventory ---------------------------------------------------

    #[test]
    fn the_service_wire_name_is_the_frozen_v3_name() {
        assert_eq!(ZONE_SERVICE_NAME, "d2b.zone.v3.ZoneService");
        assert!(!ZONE_SERVICE_NAME.contains("realm"));
    }

    #[test]
    fn every_method_has_a_distinct_kebab_wire_name() {
        let methods = [
            ZoneServiceMethod::ZoneBootstrap,
            ZoneServiceMethod::ZoneEnroll,
            ZoneServiceMethod::ResolveZoneRoute,
            ZoneServiceMethod::AuthorizeZoneShortcut,
            ZoneServiceMethod::RevokeZoneShortcut,
            ZoneServiceMethod::ReportZoneShortcutClose,
            ZoneServiceMethod::ZoneInspect,
            ZoneServiceMethod::ZoneTopologyList,
            ZoneServiceMethod::ZoneTopologyWatch,
            ZoneServiceMethod::ZoneRelayHop,
        ];
        let names: BTreeSet<&str> = methods.iter().map(|method| method.wire_name()).collect();
        assert_eq!(names.len(), methods.len());
        for name in names {
            assert_eq!(name, name.to_ascii_lowercase());
            assert!(!name.contains('_'), "wire name is kebab-case: {name}");
        }
    }

    // -- dispatch admission ------------------------------------------------

    #[test]
    fn a_method_without_a_landed_handler_is_refused_at_admission() {
        let mut server = server();
        for method in [
            ZoneServiceMethod::ZoneBootstrap,
            ZoneServiceMethod::ZoneEnroll,
        ] {
            assert!(!method.has_handler());
            assert_eq!(
                server.begin_dispatch(method).denial_reason(),
                Some(ZoneRouteFailClosedReason::PolicyDenial)
            );
        }
        assert_eq!(server.in_flight(), 0);
    }

    #[test]
    fn dispatch_admits_exactly_the_in_flight_ceiling_and_drops_the_new_request() {
        let mut server = server();
        for expected in 1..=MAX_DISPATCH_IN_FLIGHT {
            assert_eq!(
                server.begin_dispatch(ZoneServiceMethod::ResolveZoneRoute),
                ZoneDispatchAdmission::Admitted {
                    in_flight_after: expected
                }
            );
        }
        assert_eq!(server.in_flight(), MAX_DISPATCH_IN_FLIGHT);
        assert_eq!(
            server
                .begin_dispatch(ZoneServiceMethod::ResolveZoneRoute)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::QueueFullDropNew)
        );
        // Overflow drops the new request; in-flight work is untouched.
        assert_eq!(server.in_flight(), MAX_DISPATCH_IN_FLIGHT);
        server.end_dispatch();
        assert!(matches!(
            server.begin_dispatch(ZoneServiceMethod::ResolveZoneRoute),
            ZoneDispatchAdmission::Admitted { .. }
        ));
    }

    #[test]
    fn an_unbalanced_release_cannot_underflow_the_dispatch_window() {
        let mut server = server();
        server.end_dispatch();
        server.end_dispatch();
        assert_eq!(server.in_flight(), 0);
    }

    #[test]
    fn configured_bounds_refuse_zero_and_over_ceiling_values() {
        assert_eq!(
            ZoneServiceLimits::new(0, 8, 8),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            ZoneServiceLimits::new(MAX_CONFIGURED_BOUND + 1, 8, 8),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        assert_eq!(
            ZoneServiceLimits::new(8, 8, MAX_DISPATCH_IN_FLIGHT + 1),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        let limits = ZoneServiceLimits::default();
        assert_eq!(limits.max_shortcuts, DEFAULT_MAX_SHORTCUTS);
        assert_eq!(limits.audit_capacity, DEFAULT_AUDIT_CAPACITY);
        assert_eq!(limits.max_dispatch_in_flight, MAX_DISPATCH_IN_FLIGHT);
        assert_eq!(SHUTDOWN_TIMEOUT_SECONDS, 5);
    }

    // -- topology projection ----------------------------------------------

    #[test]
    fn the_projection_is_exactly_the_sealed_rows_in_child_order() {
        let server = server();
        let rows = server.list_topology(&seeded_engine(), &allowed_topology_request());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].child_zone, zone(&["k1", "k0"]));
        assert_eq!(rows[0].parent_zone, zone(&["k0"]));
        assert_eq!(rows[1].child_zone, zone(&["k2", "k1", "k0"]));
        assert_eq!(rows[1].parent_zone, zone(&["k1", "k0"]));
        for row in &rows {
            assert_eq!(row.status, ZoneTopologyStatus::Reachable);
        }
    }

    #[test]
    fn the_local_root_has_no_row_and_no_parent_store_row_exists() {
        let server = server();
        let engine = seeded_engine();
        let request = allowed_topology_request();
        assert!(
            server
                .inspect_zone(&engine, &zone(&["k0"]), &request)
                .is_none()
        );
        // A parent keeps no reciprocal row for a child it provisions, so
        // inspecting from the parent side yields nothing extra either.
        assert!(
            server
                .list_topology(&engine, &request)
                .iter()
                .all(|row| row.child_zone != zone(&["k0"]))
        );
    }

    #[test]
    fn inspecting_an_unsealed_zone_returns_no_row() {
        let server = server();
        assert!(
            server
                .inspect_zone(
                    &seeded_engine(),
                    &zone(&["unknown", "k0"]),
                    &allowed_topology_request()
                )
                .is_none()
        );
    }

    #[test]
    fn an_unauthenticated_projection_reports_every_remote_row_unreachable() {
        let server = server();
        let mut request = allowed_topology_request();
        request.route_projection_authenticated = false;
        let rows = server.list_topology(&seeded_engine(), &request);
        for row in &rows {
            assert_eq!(
                row.status,
                ZoneTopologyStatus::Unreachable {
                    reason: ZoneRouteFailClosedReason::PolicyDenial
                }
            );
        }
    }

    #[test]
    fn a_stale_projection_reports_the_row_unreachable() {
        let server = server();
        let mut request = allowed_topology_request();
        // The seeded advertisement expires at 4000.
        request.current_time_unix_seconds = 9_000;
        let row = server
            .inspect_zone(&seeded_engine(), &zone(&["k2", "k1", "k0"]), &request)
            .expect("the sealed row survives projection expiry");
        assert_eq!(
            row.status,
            ZoneTopologyStatus::Unreachable {
                reason: ZoneRouteFailClosedReason::UnknownParent
            }
        );
    }

    #[test]
    fn a_withdrawn_projection_is_indistinguishable_from_an_absent_one() {
        let server = server();
        // An engine that never admitted an advertisement stands in for the
        // post-withdrawal state; both must report the same closed reason so
        // the projection never discloses that a route once existed.
        let empty = ZoneRouteEngine::new(zone(&["k0"]));
        let row = server
            .inspect_zone(
                &empty,
                &zone(&["k2", "k1", "k0"]),
                &allowed_topology_request(),
            )
            .expect("the sealed row is still projected");
        assert_eq!(
            row.status,
            ZoneTopologyStatus::Unreachable {
                reason: ZoneRouteFailClosedReason::UnknownParent
            }
        );
    }

    #[test]
    fn projection_request_defaults_refuse() {
        let server = server();
        let request = ZoneTopologyRequest::new(1_500);
        assert!(!request.policy_allows);
        assert!(!request.zone_link_connected);
        assert!(!request.route_projection_authenticated);
        for row in server.list_topology(&seeded_engine(), &request) {
            assert!(matches!(row.status, ZoneTopologyStatus::Unreachable { .. }));
        }
    }

    // -- watch -------------------------------------------------------------

    #[test]
    fn watch_reports_once_then_only_on_change_with_a_monotonic_revision() {
        let mut server = server();
        let engine = seeded_engine();
        let request = allowed_topology_request();

        let first = server
            .poll_topology_watch(&engine, &request)
            .expect("the first poll always reports");
        assert_eq!(first.revision, 1);
        assert_eq!(first.rows.len(), 2);

        assert!(
            server.poll_topology_watch(&engine, &request).is_none(),
            "an unchanged projection reports nothing"
        );

        // Expiring the projection changes every remote row's status.
        let mut later = request;
        later.current_time_unix_seconds = 9_000;
        let second = server
            .poll_topology_watch(&engine, &later)
            .expect("a changed projection reports");
        assert_eq!(second.revision, 2);
        assert!(
            second
                .rows
                .iter()
                .any(|row| matches!(row.status, ZoneTopologyStatus::Unreachable { .. }))
        );
        assert!(server.poll_topology_watch(&engine, &later).is_none());
    }

    // -- route resolution --------------------------------------------------

    #[test]
    fn resolve_returns_the_resolver_outcome_unchanged_and_audits_it() {
        let mut server = server();
        let engine = seeded_engine();
        let request = allowed_entrypoint_request(zone(&["k2", "k1", "k0"]));
        let expected = server.resolver().resolve(&engine, &request);
        let served = server.resolve_zone_route(&engine, &request);
        assert!(served == expected);
        assert_eq!(
            server
                .audit_events()
                .last()
                .copied()
                .expect("one audit record"),
            ZoneServiceAuditEvent {
                method: ZoneServiceMethod::ResolveZoneRoute,
                kind: ZoneRouteAuditEventKind::ZoneRouteAllowed,
                denial_reason: None,
            }
        );
    }

    #[test]
    fn a_refused_resolution_is_audited_with_its_closed_reason() {
        let mut server = server();
        let engine = seeded_engine();
        let mut request = allowed_entrypoint_request(zone(&["k2", "k1", "k0"]));
        request.zone_link_connected = false;
        assert_eq!(
            server.resolve_zone_route(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
        let record = server
            .audit_events()
            .last()
            .copied()
            .expect("one audit record");
        assert_eq!(record.kind, ZoneRouteAuditEventKind::ZoneRouteDenied);
        assert_eq!(
            record.denial_reason,
            Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
    }

    // -- relay plus target verb -------------------------------------------

    #[test]
    fn a_forwarding_hop_needs_the_relay_grant_and_the_target_verb_independently() {
        let mut server = server();
        let mut base = ZoneRelayRequest::new(4);
        base.zone_link_connected = true;

        // Neither grant.
        assert_eq!(
            server.admit_relay_hop(&base).denial_reason(),
            Some(ZoneRouteFailClosedReason::RelayDenied)
        );

        // Target verb only: the relay grant is still required.
        let mut target_only = base;
        target_only.target_verb_granted = true;
        assert_eq!(
            server.admit_relay_hop(&target_only).denial_reason(),
            Some(ZoneRouteFailClosedReason::RelayDenied)
        );

        // Relay only: the target verb is still required, and the relay grant
        // does not supply it.
        let mut relay_only = base;
        relay_only.relay_granted = true;
        assert_eq!(
            server.admit_relay_hop(&relay_only).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );

        // Both grants, independently held.
        let mut both = base;
        both.relay_granted = true;
        both.target_verb_granted = true;
        assert_eq!(
            server.admit_relay_hop(&both),
            ZoneRelayAdmission::Admitted {
                forwarded_remaining_hops: 3
            }
        );
        assert_eq!(
            server
                .audit_events()
                .last()
                .copied()
                .expect("one audit record")
                .kind,
            ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted
        );
    }

    // -- shortcuts ---------------------------------------------------------

    #[test]
    fn a_shortcut_is_addressed_by_zone_path_and_authorized_only_via_resolution() {
        let mut server = server();
        let engine = seeded_engine();
        // The target is an unsealed descendant; the shortcut is addressed by
        // the resolved sealed entrypoint, not by the requested path.
        let request = allowed_entrypoint_request(zone(&["deep", "k2", "k1", "k0"]));
        let ZoneShortcutOutcome::Authorized {
            entrypoint_zone,
            remaining_hops_after,
        } = server.authorize_zone_shortcut(&engine, &request)
        else {
            panic!("expected the shortcut to be authorized");
        };
        assert_eq!(entrypoint_zone, zone(&["k2", "k1", "k0"]));
        assert_eq!(remaining_hops_after, ZONE_ROUTE_INITIAL_HOP_BUDGET - 2);
        assert!(server.shortcut_is_live(&entrypoint_zone));
        assert!(!server.shortcut_is_live(&zone(&["deep", "k2", "k1", "k0"])));
    }

    #[test]
    fn a_shortcut_is_refused_when_the_route_would_be_refused() {
        let mut server = server();
        let engine = seeded_engine();
        let mut request = allowed_entrypoint_request(zone(&["k2", "k1", "k0"]));
        request.policy_allows = false;
        assert_eq!(
            server
                .authorize_zone_shortcut(&engine, &request)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
        assert!(!server.shortcut_is_live(&zone(&["k2", "k1", "k0"])));
    }

    #[test]
    fn revocation_and_reported_close_both_clear_the_shortcut_exactly_once() {
        let mut server = server();
        let engine = seeded_engine();
        let entrypoint = zone(&["k2", "k1", "k0"]);
        let request = allowed_entrypoint_request(entrypoint.clone());

        assert!(matches!(
            server.authorize_zone_shortcut(&engine, &request),
            ZoneShortcutOutcome::Authorized { .. }
        ));
        assert_eq!(
            server.revoke_zone_shortcut(&entrypoint),
            ZoneShortcutOutcome::Closed
        );
        // A second removal is refused rather than silently succeeding.
        assert_eq!(
            server.revoke_zone_shortcut(&entrypoint).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );

        assert!(matches!(
            server.authorize_zone_shortcut(&engine, &request),
            ZoneShortcutOutcome::Authorized { .. }
        ));
        assert_eq!(
            server.report_zone_shortcut_close(&entrypoint),
            ZoneShortcutOutcome::Closed
        );
        assert_eq!(
            server
                .report_zone_shortcut_close(&entrypoint)
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );

        let kinds: Vec<ZoneRouteAuditEventKind> =
            server.audit_events().map(|event| event.kind).collect();
        assert!(kinds.contains(&ZoneRouteAuditEventKind::ZoneLinkRevoked));
        assert!(kinds.contains(&ZoneRouteAuditEventKind::ZoneLinkShortcutTornDown));
    }

    #[test]
    fn the_shortcut_table_refuses_to_grow_past_its_bound() {
        let mut server = ZoneServiceServer::with_limits(
            zone(&["k0"]),
            edges(),
            ZoneServiceLimits::new(1, 8, 4).expect("valid bounds"),
        )
        .expect("well formed topology");
        let engine = seeded_engine();

        assert!(matches!(
            server.authorize_zone_shortcut(
                &engine,
                &allowed_entrypoint_request(zone(&["k2", "k1", "k0"]))
            ),
            ZoneShortcutOutcome::Authorized { .. }
        ));
        // Re-authorizing a live entrypoint is idempotent and does not consume
        // a second slot.
        assert!(matches!(
            server.authorize_zone_shortcut(
                &engine,
                &allowed_entrypoint_request(zone(&["k2", "k1", "k0"]))
            ),
            ZoneShortcutOutcome::Authorized { .. }
        ));
        // A different entrypoint is refused rather than growing the table.
        assert_eq!(
            server
                .authorize_zone_shortcut(&engine, &allowed_entrypoint_request(zone(&["k1", "k0"])))
                .denial_reason(),
            Some(ZoneRouteFailClosedReason::QueueFullDropNew)
        );
    }

    // -- audit -------------------------------------------------------------

    #[test]
    fn the_audit_ring_is_bounded_and_drops_the_oldest_record() {
        let mut server = ZoneServiceServer::with_limits(
            zone(&["k0"]),
            edges(),
            ZoneServiceLimits::new(4, 2, 4).expect("valid bounds"),
        )
        .expect("well formed topology");
        for _ in 0..8 {
            server.begin_dispatch(ZoneServiceMethod::ZoneBootstrap);
        }
        assert_eq!(server.audit_events().len(), 2);
    }

    #[test]
    fn no_audit_record_can_carry_a_zone_path_or_a_free_form_string() {
        let mut server = server();
        let engine = seeded_engine();
        let request = allowed_entrypoint_request(zone(&["k2", "k1", "k0"]));
        server.resolve_zone_route(&engine, &request);
        for event in server.audit_events() {
            let rendered = format!("{event:?}");
            assert!(!rendered.contains("k0"), "rendered: {rendered}");
            assert!(!rendered.contains("k2"), "rendered: {rendered}");
        }
    }

    // -- redaction and construction ---------------------------------------

    #[test]
    fn public_debug_renders_no_zone_path() {
        let mut server = server();
        let engine = seeded_engine();
        let request = allowed_topology_request();
        let rows = server.list_topology(&engine, &request);
        let update = server
            .poll_topology_watch(&engine, &request)
            .expect("the first poll reports");
        let shortcut = server
            .authorize_zone_shortcut(&engine, &allowed_entrypoint_request(zone(&["k1", "k0"])));

        for rendered in [
            format!("{server:?}"),
            format!("{:?}", rows[0]),
            format!("{:?}", rows[0].status),
            format!("{update:?}"),
            format!("{shortcut:?}"),
        ] {
            assert!(rendered.contains("redacted"), "rendered: {rendered}");
            assert!(!rendered.contains("k0"), "rendered: {rendered}");
            assert!(!rendered.contains("k1"), "rendered: {rendered}");
        }
    }

    #[test]
    fn construction_rejects_a_topology_the_seal_rejects() {
        // k9.k0 is never declared as a child, so the row would attach a
        // subtree outside the sealed scope.
        assert_eq!(
            ZoneServiceServer::new(
                zone(&["k0"]),
                vec![edge(&["k9", "k0"], &["k1", "k9", "k0"])]
            )
            .err(),
            Some(PrimitiveSpecError::MissingRequiredField)
        );
    }

    #[test]
    fn a_repeated_identical_row_is_projected_once() {
        let server = ZoneServiceServer::new(
            zone(&["k0"]),
            vec![edge(&["k0"], &["k1", "k0"]), edge(&["k0"], &["k1", "k0"])],
        )
        .expect("identical rows are idempotent");
        assert_eq!(
            server
                .list_topology(&seeded_engine(), &allowed_topology_request())
                .len(),
            1
        );
    }
}
