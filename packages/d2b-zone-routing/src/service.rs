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
//! Every refusal in this file is one closed [`ZoneRouteFailClosedReason`] or
//! one closed `ZoneEnrollmentRefusal`. There is no permissive default: a
//! missing or mismatched runtime-issued admission refuses a route, a
//! method with no landed handler is refused at dispatch admission rather than
//! served, and an over-ceiling dispatch or shortcut table is refused rather
//! than grown. The service mints, holds, and presents no authority, and
//! performs no I/O.
//!
//! Nothing here accepts or returns a uid, gid, host path, socket path, store
//! path, transport endpoint, credential, or key material.
//!
//! # Enrollment
//!
//! `zone-bootstrap` and `zone-enroll` are landed. Both consume a
//! runtime-issued, single-use [`ZoneEnrollmentAdmission`] and compose the
//! frozen ZoneLink enrollment state machine rather than restating it:
//! `zone-bootstrap` admits one IKpsk2 attempt and burns the presented
//! single-use PSK, and `zone-enroll` seals the enrollment record and admits
//! the peer into the enrolled session, answering with the link epoch the
//! session was assigned. The admitted route a Guest agent receives is the
//! allocator's own tuple - the placed `ZoneId` and the enrollment generation -
//! never a credential, a key, or a handle.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2b_bus::session::{
    BootstrapPskIssuance, EnrollmentFingerprint, EnrollmentRecord, RouteAdmissionEvidence,
    RouteAdmissionVerifier, ZoneLinkEnrollment, ZoneLinkEnrollmentError, ZoneLinkState,
};
use d2b_contracts_resource::v3::execution_policy::PrimitiveSpecError;
use d2b_contracts_zone_session::v3::zone_routing::{
    MAX_ZONE_PARENT_ENTRIES, ZONE_ROUTE_INITIAL_HOP_BUDGET, ZonePath, ZoneRouteAuditEventKind,
    ZoneRouteFailClosedReason, ZoneTreeEdge,
};
use d2b_contracts_zone_session::v3::zone_session::{
    ZONE_BOOTSTRAP_METHOD, ZONE_ENROLL_METHOD, ZoneBootstrapCall, ZoneBootstrapReply,
    ZoneEnrollCall, ZoneEnrollReply, ZoneEnrollmentIdentity, ZoneEnrollmentRefusal,
};

use crate::engine::{
    ZoneRelayAdmission, ZoneRelayRequest, ZoneRouteAdmission, ZoneRouteAdmissionExpectation,
    ZoneRouteEngine,
};
use crate::enrollment::{
    ZoneEnrollmentAdmission, ZoneEnrollmentAdmissionEvidence, ZoneEnrollmentAdmissionVerifier,
    ZoneEnrollmentExpectation,
};
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
/// The inventory is frozen here in full. Every method has a landed handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneServiceMethod {
    /// One-time IKpsk2 bootstrap consuming the allocator-issued single-use
    /// PSK.
    ZoneBootstrap,
    /// Noise_KK enrollment following a consumed bootstrap.
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
    ///
    /// The two enrollment method names are the frozen contract constants
    /// rather than a second copy of the spelling, so the service, the
    /// allocator, and a Guest agent client cannot drift apart.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::ZoneBootstrap => ZONE_BOOTSTRAP_METHOD,
            Self::ZoneEnroll => ZONE_ENROLL_METHOD,
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

/// The runtime-issued admissions used to evaluate a topology projection.
///
/// Each sealed child row is keyed to its own admission. Missing evidence
/// leaves that row unreachable; there is no shared caller-populated policy,
/// connectivity, authentication, capability, or time flag.
pub struct ZoneTopologyRequest {
    admissions: std::collections::BTreeMap<ZonePath, ZoneRouteAdmission>,
}

impl ZoneTopologyRequest {
    /// A projection request with no runtime admissions.
    pub fn new() -> Self {
        Self {
            admissions: std::collections::BTreeMap::new(),
        }
    }

    /// Attach the admission bound to one sealed child Zone.
    pub fn with_admission(mut self, child_zone: ZonePath, admission: ZoneRouteAdmission) -> Self {
        self.admissions.insert(child_zone, admission);
        self
    }

    /// Consume and verify the admission bound to one sealed child Zone.
    pub fn with_runtime_admission(
        self,
        child_zone: ZonePath,
        verifier: RouteAdmissionVerifier,
        evidence: RouteAdmissionEvidence,
        expected: &ZoneRouteAdmissionExpectation,
    ) -> Result<Self, ZoneRouteFailClosedReason> {
        let admission = ZoneRouteAdmission::verify(verifier, evidence, expected)?;
        Ok(self.with_admission(child_zone, admission))
    }

    /// Borrow the admission bound to one child Zone, when present.
    pub fn admission_for(&self, child_zone: &ZonePath) -> Option<&ZoneRouteAdmission> {
        self.admissions.get(child_zone)
    }
}

impl Default for ZoneTopologyRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ZoneTopologyRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneTopologyRequest(<redacted>)")
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

/// One `zone-bootstrap` call as this Zone serves it.
///
/// The wire [`ZoneBootstrapCall`] is what a Guest agent sends. The admission
/// and the daemon time are attached by the serving runtime, exactly as the
/// topology and entrypoint requests attach theirs: neither is on the wire and
/// neither is the caller's to state.
pub struct ZoneBootstrapRequest {
    admission: Option<ZoneEnrollmentAdmission>,
    now_unix_ms: u64,
    call: ZoneBootstrapCall,
}

impl ZoneBootstrapRequest {
    /// A bootstrap request with no runtime admission.
    pub const fn new(call: ZoneBootstrapCall, now_unix_ms: u64) -> Self {
        Self {
            admission: None,
            now_unix_ms,
            call,
        }
    }

    /// Attach the runtime-issued admission this Zone holds for the link.
    pub fn with_admission(mut self, admission: ZoneEnrollmentAdmission) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Consume and verify one runtime-issued admission for this request.
    pub fn with_runtime_admission(
        self,
        verifier: ZoneEnrollmentAdmissionVerifier,
        evidence: ZoneEnrollmentAdmissionEvidence,
        expected: &ZoneEnrollmentExpectation,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        let admission = ZoneEnrollmentAdmission::verify(verifier, evidence, expected)?;
        Ok(self.with_admission(admission))
    }

    /// Borrow the wire call.
    pub const fn call(&self) -> &ZoneBootstrapCall {
        &self.call
    }
}

impl std::fmt::Debug for ZoneBootstrapRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneBootstrapRequest(<redacted>)")
    }
}

/// One `zone-enroll` call as this Zone serves it.
pub struct ZoneEnrollRequest {
    admission: Option<ZoneEnrollmentAdmission>,
    now_unix_ms: u64,
    call: ZoneEnrollCall,
}

impl ZoneEnrollRequest {
    /// An enrollment request with no runtime admission.
    pub const fn new(call: ZoneEnrollCall, now_unix_ms: u64) -> Self {
        Self {
            admission: None,
            now_unix_ms,
            call,
        }
    }

    /// Attach the runtime-issued admission this Zone holds for the link.
    pub fn with_admission(mut self, admission: ZoneEnrollmentAdmission) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Consume and verify one runtime-issued admission for this request.
    pub fn with_runtime_admission(
        self,
        verifier: ZoneEnrollmentAdmissionVerifier,
        evidence: ZoneEnrollmentAdmissionEvidence,
        expected: &ZoneEnrollmentExpectation,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        let admission = ZoneEnrollmentAdmission::verify(verifier, evidence, expected)?;
        Ok(self.with_admission(admission))
    }

    /// Borrow the wire call.
    pub const fn call(&self) -> &ZoneEnrollCall {
        &self.call
    }
}

impl std::fmt::Debug for ZoneEnrollRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollRequest(<redacted>)")
    }
}

/// One admitted enrollment call, after its admission was consumed.
struct AdmittedEnrollment {
    expected: ZoneEnrollmentExpectation,
    child: ZonePath,
}

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
    /// The one Zone this service instance serves.
    local_root: ZonePath,
    /// The ZoneLink enrollment state machine of every sealed child link.
    ///
    /// The composition is the frozen five-state machine; this server owns no
    /// transition rule of its own and never restates one.
    links: BTreeMap<ZonePath, ZoneLinkEnrollment>,
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
        let local_root = topology.local_root().clone();
        Ok(Self {
            resolver: ZoneEntrypointResolver::new(topology),
            rows,
            local_root,
            links: BTreeMap::new(),
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
    /// Refuses a request beyond the concurrency ceiling with
    /// [`ZoneRouteFailClosedReason::QueueFullDropNew`]. Overflow drops the new
    /// request rather than displacing in-flight work, matching the engine's
    /// pre-authentication admission. Every method in the frozen inventory has
    /// a landed handler; the handlers themselves are what refuse a request
    /// that carries no admissible authority.
    pub fn begin_dispatch(&mut self, method: ZoneServiceMethod) -> ZoneDispatchAdmission {
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

    /// Serve `zone-bootstrap`.
    ///
    /// The handler composes the frozen enrollment state machine: it consumes the
    /// runtime-issued admission, refuses a call whose identity is not the identity
    /// that admission was issued for, requires the named edge to be one of this
    /// Zone's sealed rows, and only then admits one IKpsk2 attempt - which burns
    /// the presented single-use PSK whether or not the handshake that follows
    /// succeeds.
    pub fn zone_bootstrap(&mut self, request: &ZoneBootstrapRequest) -> ZoneBootstrapReply {
        let method = ZoneServiceMethod::ZoneBootstrap;
        let call = &request.call;
        let admitted = match self.admit_enrollment(&call.identity, request.admission.as_ref()) {
            Ok(admitted) => admitted,
            Err(refusal) => return self.refuse_enrollment(method, refusal),
        };

        let psk =
            match BootstrapPskIssuance::new(call.issuance, call.issued_at_unix_ms, call.ttl_ms) {
                Ok(psk) => psk,
                Err(error) => return self.refuse_enrollment(method, machine_refusal(error)),
            };

        let child = admitted.child.clone();
        let tracked = self.links.contains_key(&child);
        let mut link = self
            .links
            .remove(&child)
            .unwrap_or_else(ZoneLinkEnrollment::new_unenrolled);
        match link.begin_bootstrap(psk, request.now_unix_ms) {
            Ok(()) => {
                self.links.insert(child, link);
                self.record(
                    method,
                    ZoneRouteAuditEventKind::ZoneLinkSessionEstablished,
                    None,
                );
                ZoneBootstrapReply::Admitted {
                    expires_at_unix_ms: call.issued_at_unix_ms.saturating_add(call.ttl_ms),
                }
            }
            Err(error) => {
                // A refused bootstrap leaves no state behind for a link that
                // had none; a link that was already tracked keeps its state.
                if tracked {
                    self.links.insert(child, link);
                }
                self.refuse_enrollment(method, machine_refusal(error))
            }
        }
    }

    /// Serve `zone-enroll`.
    ///
    /// The handler consumes the runtime-issued admission exactly as
    /// `zone-bootstrap` does, then seals the enrollment record and admits the
    /// peer into the enrolled `Noise_KK` session, answering with the Zone the
    /// allocator placed the agent in and the link epoch the session was
    /// assigned. A peer whose fingerprint does not match the sealed record is
    /// refused and the link returns to `EnrollmentCommitted` rather than
    /// downgrading.
    pub fn zone_enroll(&mut self, request: &ZoneEnrollRequest) -> ZoneEnrollReply {
        let method = ZoneServiceMethod::ZoneEnroll;
        let call = &request.call;
        let admitted = match self.admit_enrollment(&call.identity, request.admission.as_ref()) {
            Ok(admitted) => admitted,
            Err(refusal) => return self.refuse_enroll(method, refusal),
        };

        let Some(fingerprint) = EnrollmentFingerprint::new(call.observed_peer_fingerprint) else {
            return self.refuse_enroll(method, ZoneEnrollmentRefusal::MalformedRequest);
        };
        let Some(sealed) =
            EnrollmentFingerprint::new(admitted.expected.enrolled_peer_fingerprint())
        else {
            return self.refuse_enroll(method, ZoneEnrollmentRefusal::MalformedRequest);
        };
        // The record is sealed from the allocator's own tuple, never from
        // anything the request states, so a peer cannot enroll itself against
        // a fingerprint it chose.
        let record = EnrollmentRecord::new(sealed, admitted.expected.allocator_binding());

        let child = admitted.child.clone();
        let tracked = self.links.contains_key(&child);
        let mut link = self
            .links
            .remove(&child)
            .unwrap_or_else(ZoneLinkEnrollment::new_unenrolled);
        let outcome = if link.state() == ZoneLinkState::EnrollmentCommitted {
            // A peer refused after the record was sealed needs no second
            // commit: the same sealed record admits it on the next enrolled
            // handshake, and there is no path from here back to a bootstrap.
            link.begin_enrolled_handshake()
                .and_then(|()| link.establish(fingerprint, request.now_unix_ms))
        } else {
            link.commit_enrollment(record)
                .and_then(|()| link.begin_enrolled_handshake())
                .and_then(|()| link.establish(fingerprint, request.now_unix_ms))
        };
        match outcome {
            Ok(epoch) => {
                self.links.insert(child, link);
                self.record(
                    method,
                    ZoneRouteAuditEventKind::ZoneLinkSessionEstablished,
                    None,
                );
                ZoneEnrollReply::Enrolled {
                    zone: admitted.expected.zone().clone(),
                    generation: epoch.get(),
                }
            }
            Err(error) => {
                // A peer that presented the wrong key falls back to the
                // committed enrollment rather than to a bootstrap, so a link
                // that was already tracked keeps its advanced state.
                if tracked {
                    self.links.insert(child, link);
                }
                self.refuse_enroll(method, machine_refusal(error))
            }
        }
    }

    /// The current enrollment state of one sealed child link, when known.
    pub fn link_state(&self, child: &ZonePath) -> Option<ZoneLinkState> {
        self.links.get(child).map(ZoneLinkEnrollment::state)
    }

    /// Consume the admission and check the call against this Zone's truth.
    ///
    /// The tuple every check below reads is the one the consumed evidence was
    /// sealed to, so a request can only be admitted against the seal.
    ///
    /// Every refusal returns without recording; the caller records exactly
    /// once through [`Self::refuse_enrollment`] or [`Self::refuse_enroll`].
    fn admit_enrollment(
        &mut self,
        identity: &ZoneEnrollmentIdentity,
        admission: Option<&ZoneEnrollmentAdmission>,
    ) -> Result<AdmittedEnrollment, ZoneEnrollmentRefusal> {
        let expected = admission
            .ok_or(ZoneEnrollmentRefusal::AdmissionAbsent)?
            .consume()?;
        identity.validate()?;
        if !expected.admits(identity) {
            // A request that names a link but not the session profile it was
            // admitted for is refused distinctly: the link identity matched,
            // so this is a stale or substituted session binding rather than a
            // substituted link.
            return Err(if expected.admits_link(identity) {
                ZoneEnrollmentRefusal::SessionProfileRefused
            } else {
                ZoneEnrollmentRefusal::IdentityMismatch
            });
        }
        let child = expected.edge().child();
        let sealed = expected.edge().parent() == &self.local_root
            && self.rows.iter().any(|(row_child, row_parent)| {
                row_child == child && row_parent == expected.edge().parent()
            });
        if !sealed {
            return Err(ZoneEnrollmentRefusal::UnsealedZoneLink);
        }
        Ok(AdmittedEnrollment {
            child: child.clone(),
            expected,
        })
    }

    fn refuse_enrollment(
        &mut self,
        method: ZoneServiceMethod,
        refusal: ZoneEnrollmentRefusal,
    ) -> ZoneBootstrapReply {
        self.record(
            method,
            ZoneRouteAuditEventKind::ZoneLinkSessionFailed,
            Some(audit_reason(refusal)),
        );
        ZoneBootstrapReply::Refused { reason: refusal }
    }

    fn refuse_enroll(
        &mut self,
        method: ZoneServiceMethod,
        refusal: ZoneEnrollmentRefusal,
    ) -> ZoneEnrollReply {
        self.record(
            method,
            ZoneRouteAuditEventKind::ZoneLinkSessionFailed,
            Some(audit_reason(refusal)),
        );
        ZoneEnrollReply::Refused { reason: refusal }
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
        let resolution = match request.admission_for(child) {
            Some(admission) => self.resolver.resolve_with_admission(
                engine,
                child,
                ZONE_ROUTE_INITIAL_HOP_BUDGET,
                admission,
            ),
            None => {
                let entrypoint = ZoneEntrypointRequest::new(child.clone())
                    .with_remaining_hops(ZONE_ROUTE_INITIAL_HOP_BUDGET);
                self.resolver.resolve(engine, &entrypoint)
            }
        };

        let status = match resolution {
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

/// The closed enrollment refusal of one state-machine refusal.
///
/// The mapping is exhaustive, so a new machine refusal cannot be silently
/// swallowed into a permissive default.
fn machine_refusal(error: ZoneLinkEnrollmentError) -> ZoneEnrollmentRefusal {
    match error {
        ZoneLinkEnrollmentError::BootstrapPskConsumed => {
            ZoneEnrollmentRefusal::BootstrapPskConsumed
        }
        ZoneLinkEnrollmentError::BootstrapPskExpired => ZoneEnrollmentRefusal::BootstrapPskExpired,
        ZoneLinkEnrollmentError::BootstrapHandshakeFailed => {
            ZoneEnrollmentRefusal::BootstrapHandshakeFailed
        }
        ZoneLinkEnrollmentError::ZoneLinkEnrollmentKeyMismatch => {
            ZoneEnrollmentRefusal::ZoneLinkEnrollmentKeyMismatch
        }
        ZoneLinkEnrollmentError::ZoneLinkRevoked => ZoneEnrollmentRefusal::ZoneLinkRevoked,
        ZoneLinkEnrollmentError::InvalidTransition => ZoneEnrollmentRefusal::InvalidTransition,
        ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange => {
            ZoneEnrollmentRefusal::BootstrapPskTtlOutOfRange
        }
        ZoneLinkEnrollmentError::KkSessionLifetimeOutOfRange => {
            ZoneEnrollmentRefusal::KkSessionLifetimeOutOfRange
        }
        ZoneLinkEnrollmentError::LinkEpochExhausted => ZoneEnrollmentRefusal::LinkEpochExhausted,
        ZoneLinkEnrollmentError::ResourceTrafficBeforeReady => {
            ZoneEnrollmentRefusal::ResourceTrafficBeforeReady
        }
    }
}

/// The closed audit reason one enrollment refusal is recorded under.
fn audit_reason(refusal: ZoneEnrollmentRefusal) -> ZoneRouteFailClosedReason {
    match refusal {
        ZoneEnrollmentRefusal::AdmissionAbsent
        | ZoneEnrollmentRefusal::PolicyDenial
        | ZoneEnrollmentRefusal::IdentityMismatch
        | ZoneEnrollmentRefusal::SessionProfileRefused
        | ZoneEnrollmentRefusal::ZoneLinkEnrollmentKeyMismatch
        | ZoneEnrollmentRefusal::ZoneLinkRevoked
        | ZoneEnrollmentRefusal::InvalidTransition
        | ZoneEnrollmentRefusal::BootstrapPskTtlOutOfRange
        | ZoneEnrollmentRefusal::KkSessionLifetimeOutOfRange
        | ZoneEnrollmentRefusal::LinkEpochExhausted
        | ZoneEnrollmentRefusal::MalformedRequest => ZoneRouteFailClosedReason::PolicyDenial,
        ZoneEnrollmentRefusal::AdmissionConsumed | ZoneEnrollmentRefusal::BootstrapPskConsumed => {
            ZoneRouteFailClosedReason::Replay
        }
        ZoneEnrollmentRefusal::AdmissionExpired | ZoneEnrollmentRefusal::BootstrapPskExpired => {
            ZoneRouteFailClosedReason::Expired
        }
        ZoneEnrollmentRefusal::UnsealedZoneLink => ZoneRouteFailClosedReason::UnknownParent,
        ZoneEnrollmentRefusal::BootstrapHandshakeFailed
        | ZoneEnrollmentRefusal::ResourceTrafficBeforeReady => {
            ZoneRouteFailClosedReason::ZoneLinkDisconnected
        }
        ZoneEnrollmentRefusal::PayloadTooLarge => ZoneRouteFailClosedReason::QueueFullDropNew,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use d2b_contracts_resource::v3::{
        ResourceUid, ZoneId, ZoneRevision, identity::ReconnectGeneration,
    };
    use d2b_contracts_zone_session::v3::component_session::LimitProfile;
    use d2b_contracts_zone_session::v3::{
        component_session::{OperationClass, OperationId},
        zone_routing::{
            ZONE_ROUTING_SCHEMA_VERSION, ZoneDescendantRoute, ZoneLabelId,
            ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation, ZoneLinkRouteAdvertisement,
            ZoneRouteCapability, ZoneRouteCapabilitySet, ZoneRouteId, ZoneRouteKeyRole,
            ZoneRouteSignature, ZoneRouteSignatureAlgorithm, ZoneRouteSignatureRef,
            ZoneSigningKeyFingerprint,
        },
    };

    use crate::engine::{
        ZoneAdvertisementAdmission, ZoneRouteAdmission, ZoneRouteAdmissionExpectation,
    };
    use crate::enrollment::ZoneEnrollmentAuthority;

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

    fn uid(marker: char) -> ResourceUid {
        let value = match marker {
            '1' => "11111111-1111-4111-8111-111111111111",
            '2' => "22222222-2222-4222-8222-222222222222",
            '3' => "33333333-3333-4333-8333-333333333333",
            _ => panic!("test UID marker must be one of 1..=3"),
        };
        ResourceUid::parse(value).expect("valid resource UID")
    }

    fn admission(capability: &str, issued_at: u64, expires_at: u64) -> ZoneRouteAdmission {
        admission_for_zones(
            zone(&["k0"]),
            zone(&["k2", "k1", "k0"]),
            OperationClass::Invoke,
            capability,
            issued_at,
            expires_at,
        )
    }

    fn admission_for(
        verb: OperationClass,
        capability: &str,
        issued_at: u64,
        expires_at: u64,
    ) -> ZoneRouteAdmission {
        admission_for_zones(
            zone(&["k0"]),
            zone(&["k2", "k1", "k0"]),
            verb,
            capability,
            issued_at,
            expires_at,
        )
    }

    fn admission_for_zones(
        source: ZonePath,
        target: ZonePath,
        verb: OperationClass,
        capability: &str,
        issued_at: u64,
        expires_at: u64,
    ) -> ZoneRouteAdmission {
        let expectation = ZoneRouteAdmissionExpectation::new(
            uid('1'),
            edge(&["k0"], &["k1", "k0"]),
            ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
            ReconnectGeneration::new(7).expect("valid reconnect generation"),
            uid('2'),
            uid('3'),
            OperationId::new(vec![0x11; 16]).expect("valid operation ID"),
            verb,
            ZoneRouteCapability::parse(capability).expect("valid capability"),
            ZoneRevision::new(9),
        )
        .expect("valid route admission expectation")
        .for_zones(source, target);
        ZoneRouteAdmission::for_test(expectation, issued_at, expires_at)
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
        ZoneTopologyRequest::new()
            .with_admission(
                zone(&["k1", "k0"]),
                admission_for_zones(
                    zone(&["k0"]),
                    zone(&["k1", "k0"]),
                    OperationClass::Invoke,
                    "get",
                    1_500,
                    4_000,
                ),
            )
            .with_admission(zone(&["k2", "k1", "k0"]), admission("get", 1_500, 4_000))
    }

    fn allowed_entrypoint_request(target: ZonePath) -> ZoneEntrypointRequest {
        let source = zone(&["k0"]);
        ZoneEntrypointRequest::new(target.clone()).with_admission(admission_for_zones(
            source,
            target,
            OperationClass::Invoke,
            "get",
            1_500,
            4_000,
        ))
    }

    // -- wire inventory ---------------------------------------------------

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
    fn every_landed_method_dispatches_within_the_window() {
        let mut server = server();
        for method in [
            ZoneServiceMethod::ZoneBootstrap,
            ZoneServiceMethod::ZoneEnroll,
            ZoneServiceMethod::ResolveZoneRoute,
        ] {
            assert!(matches!(
                server.begin_dispatch(method),
                ZoneDispatchAdmission::Admitted { .. }
            ));
            server.end_dispatch();
        }
        assert_eq!(
            ZoneServiceMethod::ZoneBootstrap.wire_name(),
            ZONE_BOOTSTRAP_METHOD
        );
        assert_eq!(
            ZoneServiceMethod::ZoneEnroll.wire_name(),
            ZONE_ENROLL_METHOD
        );
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
        let request = ZoneTopologyRequest::new();
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
        // The seeded advertisement expires at 4000; the runtime-issued
        // admission is stamped after that window.
        let request = ZoneTopologyRequest::new()
            .with_admission(zone(&["k2", "k1", "k0"]), admission("get", 9_000, 10_000));
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
        let request = ZoneTopologyRequest::new();
        assert!(request.admissions.is_empty());
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

        let unchanged_request = allowed_topology_request();
        assert!(
            server
                .poll_topology_watch(&engine, &unchanged_request)
                .is_none(),
            "an unchanged projection reports nothing"
        );

        // Expiring the projection changes every remote row's status.
        let later = ZoneTopologyRequest::new()
            .with_admission(zone(&["k1", "k0"]), admission("get", 9_000, 10_000));
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
        let later_again = ZoneTopologyRequest::new()
            .with_admission(zone(&["k1", "k0"]), admission("get", 9_000, 10_000));
        assert!(server.poll_topology_watch(&engine, &later_again).is_none());
    }

    #[test]
    fn a_topology_watch_request_cannot_reuse_consumed_admissions() {
        let mut server = server();
        let engine = seeded_engine();
        let request = allowed_topology_request();

        assert!(server.poll_topology_watch(&engine, &request).is_some());
        let reused = server
            .poll_topology_watch(&engine, &request)
            .expect("reusing consumed evidence must report a refusal");
        assert!(reused.rows.iter().all(|row| {
            matches!(
                row.status,
                ZoneTopologyStatus::Unreachable {
                    reason: ZoneRouteFailClosedReason::ZoneLinkDisconnected
                        | ZoneRouteFailClosedReason::PolicyDenial
                }
            )
        }));
    }

    // -- route resolution --------------------------------------------------

    #[test]
    fn resolve_returns_the_resolver_outcome_unchanged_and_audits_it() {
        let mut server = server();
        let engine = seeded_engine();
        let expected = server.resolver().resolve(
            &engine,
            &allowed_entrypoint_request(zone(&["k2", "k1", "k0"])),
        );
        let served = server.resolve_zone_route(
            &engine,
            &allowed_entrypoint_request(zone(&["k2", "k1", "k0"])),
        );
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
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]));
        assert_eq!(
            server.resolve_zone_route(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
        let record = server
            .audit_events()
            .last()
            .copied()
            .expect("one audit record");
        assert_eq!(record.kind, ZoneRouteAuditEventKind::ZoneRouteDenied);
        assert_eq!(
            record.denial_reason,
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    // -- relay plus target verb -------------------------------------------

    #[test]
    fn a_forwarding_hop_needs_the_relay_grant_and_the_target_verb_independently() {
        let mut server = server();
        let base = ZoneRelayRequest::new(4);

        // Neither runtime admission.
        assert_eq!(
            server.admit_relay_hop(&base).denial_reason(),
            Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );

        // Target admission only: the relay admission remains independent.
        let target_only =
            ZoneRelayRequest::new(4).with_target_admission(admission("get", 1_500, 4_000));
        assert_eq!(
            server.admit_relay_hop(&target_only),
            ZoneRelayAdmission::Denied {
                reason: ZoneRouteFailClosedReason::ZoneLinkDisconnected
            }
        );

        // Both independently verified admissions are required.
        let both = ZoneRelayRequest::new(4).with_admissions(
            admission("get", 1_500, 4_000),
            admission_for(OperationClass::Relay, "relay", 1_500, 4_000),
        );
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
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]));
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

        assert!(matches!(
            server
                .authorize_zone_shortcut(&engine, &allowed_entrypoint_request(entrypoint.clone())),
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
            server
                .authorize_zone_shortcut(&engine, &allowed_entrypoint_request(entrypoint.clone())),
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

    // -- enrollment --------------------------------------------------------

    const BOOTSTRAP_TTL_MS: u64 = 300_000;
    const ENROLL_NOW_MS: u64 = 1_700_000_000_500;
    const SEALED_PEER_FINGERPRINT: [u8; 32] = [0x33; 32];

    fn enrolled_expectation() -> ZoneEnrollmentExpectation {
        ZoneEnrollmentExpectation::for_enrolled_guest_session(
            ZoneId::parse("zone-k1").expect("valid zone"),
            uid('1'),
            edge(&["k0"], &["k1", "k0"]),
            ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
            SEALED_PEER_FINGERPRINT,
            [0x44; 32],
            [0x11; 32],
            [0x22; 32],
            ReconnectGeneration::new(7).expect("valid generation"),
            LimitProfile::remote_default(),
        )
        .expect("the contract enrolled guest session profile")
    }

    /// The same link with a differing expected reconnect generation.
    fn substituted_expectation() -> ZoneEnrollmentExpectation {
        ZoneEnrollmentExpectation::for_enrolled_guest_session(
            ZoneId::parse("zone-k1").expect("valid zone"),
            uid('1'),
            edge(&["k0"], &["k1", "k0"]),
            ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
            SEALED_PEER_FINGERPRINT,
            [0x44; 32],
            [0x11; 32],
            [0x22; 32],
            ReconnectGeneration::new(8).expect("valid generation"),
            LimitProfile::remote_default(),
        )
        .expect("the contract enrolled guest session profile")
    }

    fn enrollment_identity(expected: &ZoneEnrollmentExpectation) -> ZoneEnrollmentIdentity {
        ZoneEnrollmentIdentity {
            zone_link_uid: expected.zone_link_uid().clone(),
            edge: expected.edge().clone(),
            controller_generation: expected.controller_generation().clone(),
            reconnect_generation: ReconnectGeneration::new(
                expected.session_policy().reconnect_generation,
            )
            .expect("valid generation"),
            schema_fingerprint: expected.session_policy().schema_fingerprint,
        }
    }

    fn bootstrap_request(
        expected: &ZoneEnrollmentExpectation,
        issuance: u64,
    ) -> ZoneBootstrapRequest {
        ZoneBootstrapRequest::new(
            ZoneBootstrapCall::new(
                enrollment_identity(expected),
                issuance,
                BOOTSTRAP_TTL_MS,
                1_700_000_000_000,
            ),
            ENROLL_NOW_MS,
        )
    }

    fn enroll_request(
        expected: &ZoneEnrollmentExpectation,
        fingerprint: [u8; 32],
    ) -> ZoneEnrollRequest {
        ZoneEnrollRequest::new(
            ZoneEnrollCall::new(enrollment_identity(expected), fingerprint, ENROLL_NOW_MS),
            ENROLL_NOW_MS,
        )
    }

    fn admitted_bootstrap(server: &mut ZoneServiceServer, expected: &ZoneEnrollmentExpectation) {
        let bootstrap = bootstrap_request(expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert!(matches!(
            server.zone_bootstrap(&bootstrap),
            ZoneBootstrapReply::Admitted { .. }
        ));
    }

    #[test]
    fn a_bootstrap_without_a_runtime_admission_is_refused_fail_closed() {
        let mut server = server();
        let expected = enrolled_expectation();
        assert_eq!(
            server.zone_bootstrap(&bootstrap_request(&expected, 1)),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::AdmissionAbsent
            }
        );
        assert_eq!(
            server.zone_enroll(&enroll_request(&expected, [0x33; 32])),
            ZoneEnrollReply::Refused {
                reason: ZoneEnrollmentRefusal::AdmissionAbsent
            }
        );
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
        assert_eq!(server.audit_events().len(), 2);
        for event in server.audit_events() {
            assert_eq!(event.kind, ZoneRouteAuditEventKind::ZoneLinkSessionFailed);
            assert_eq!(
                event.denial_reason,
                Some(ZoneRouteFailClosedReason::PolicyDenial)
            );
        }
    }

    #[test]
    fn bootstrap_admits_once_and_burns_the_psk_against_replay() {
        let mut server = server();
        let expected = enrolled_expectation();
        let request = bootstrap_request(&expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&request),
            ZoneBootstrapReply::Admitted {
                expires_at_unix_ms: 1_700_000_300_000
            }
        );
        assert_eq!(
            server.link_state(&zone(&["k1", "k0"])),
            Some(d2b_bus::session::ZoneLinkState::IKpsk2)
        );

        // The admission was consumed, so the same request cannot be replayed.
        assert_eq!(
            server.zone_bootstrap(&request),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::AdmissionConsumed
            }
        );

        // A fresh admission cannot restart a bootstrap that is already in
        // flight: the PSK was burned by the first attempt, and the link never
        // returns to `Unenrolled` on its own.
        let replay = bootstrap_request(&expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&replay),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::InvalidTransition
            }
        );
        assert_eq!(
            server.link_state(&zone(&["k1", "k0"])),
            Some(d2b_bus::session::ZoneLinkState::IKpsk2)
        );
    }

    #[test]
    fn a_call_that_names_a_substituted_link_or_session_is_refused_before_the_psk_burns() {
        let mut server = server();
        let expected = enrolled_expectation();

        let mut substituted = enrollment_identity(&expected);
        substituted.zone_link_uid = uid('2');
        let call = ZoneBootstrapRequest::new(
            ZoneBootstrapCall::new(substituted, 1, BOOTSTRAP_TTL_MS, 1_700_000_000_000),
            ENROLL_NOW_MS,
        )
        .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&call),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::IdentityMismatch
            }
        );

        let mut substituted = enrollment_identity(&expected);
        substituted.schema_fingerprint = [0x99; 32];
        let call = ZoneBootstrapRequest::new(
            ZoneBootstrapCall::new(substituted, 1, BOOTSTRAP_TTL_MS, 1_700_000_000_000),
            ENROLL_NOW_MS,
        )
        .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&call),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::SessionProfileRefused
            }
        );

        // The admitted link never advanced, so the honest call still works.
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
        let honest = bootstrap_request(&expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert!(matches!(
            server.zone_bootstrap(&honest),
            ZoneBootstrapReply::Admitted { .. }
        ));
    }

    #[test]
    fn an_edge_outside_the_sealed_topology_is_refused() {
        let mut server = server();
        let expected = ZoneEnrollmentExpectation::for_enrolled_guest_session(
            ZoneId::parse("zone-k9").expect("valid zone"),
            uid('1'),
            edge(&["k0"], &["k9", "k0"]),
            ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
            SEALED_PEER_FINGERPRINT,
            [0x44; 32],
            [0x11; 32],
            [0x22; 32],
            ReconnectGeneration::new(7).expect("valid generation"),
            LimitProfile::remote_default(),
        )
        .expect("the contract enrolled guest session profile");
        let call = bootstrap_request(&expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&call),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::UnsealedZoneLink
            }
        );
        assert_eq!(server.audit_events().len(), 1);
        assert_eq!(
            server
                .audit_events()
                .next()
                .expect("one record")
                .denial_reason,
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_psk_lifetime_outside_the_frozen_range_is_refused() {
        let mut server = server();
        let expected = enrolled_expectation();
        let call = ZoneBootstrapRequest::new(
            ZoneBootstrapCall::new(enrollment_identity(&expected), 1, 1_000, 1_700_000_000_000),
            ENROLL_NOW_MS,
        )
        .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&call),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::BootstrapPskTtlOutOfRange
            }
        );
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
    }

    #[test]
    fn an_expired_psk_is_refused_and_leaves_the_link_unenrolled() {
        let mut server = server();
        let expected = enrolled_expectation();
        let call = ZoneBootstrapRequest::new(
            ZoneBootstrapCall::new(
                enrollment_identity(&expected),
                1,
                BOOTSTRAP_TTL_MS,
                1_700_000_000_000,
            ),
            // One millisecond past the issuance expiry.
            1_700_000_000_000 + BOOTSTRAP_TTL_MS,
        )
        .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&call),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::BootstrapPskExpired
            }
        );
        // An expired PSK is refused, not burned: the link stays unenrolled and
        // a fresh issuance is what a retry needs.
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
        let fresh = bootstrap_request(&expected, 2)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert!(matches!(
            server.zone_bootstrap(&fresh),
            ZoneBootstrapReply::Admitted { .. }
        ));
    }

    #[test]
    fn a_runtime_issued_admission_reaches_the_handler_and_then_burns() {
        let mut server = server();
        let expected = enrolled_expectation();
        let now = Arc::new(AtomicU64::new(1_700_000_000_400));
        let clock: Arc<dyn Fn() -> u64 + Send + Sync> = {
            let now = Arc::clone(&now);
            Arc::new(move || now.load(Ordering::Acquire))
        };
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock, 30_000).expect("valid authority");
        let (verifier, evidence) = authority.issue(expected.clone()).expect("issued");
        let request = bootstrap_request(&expected, 1)
            .with_runtime_admission(verifier, evidence, &expected)
            .expect("admission");
        assert!(matches!(
            server.zone_bootstrap(&request),
            ZoneBootstrapReply::Admitted { .. }
        ));
    }

    #[test]
    fn a_substituted_expectation_is_refused_at_the_handler() {
        let mut server = server();
        // The allocator sealed one tuple; the runtime holds the evidence
        // against a different one and the call names that different one. The
        // handler decides on the seal, so the call is refused even though it
        // matches the tuple it was presented with.
        let sealed = enrolled_expectation();
        let substituted = substituted_expectation();
        assert_ne!(sealed, substituted, "the two tuples differ");

        let now = Arc::new(AtomicU64::new(1_700_000_000_400));
        let clock: Arc<dyn Fn() -> u64 + Send + Sync> = {
            let now = Arc::clone(&now);
            Arc::new(move || now.load(Ordering::Acquire))
        };
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock, 30_000).expect("valid authority");
        let (verifier, evidence) = authority.issue(sealed.clone()).expect("issued");
        let bootstrap = bootstrap_request(&substituted, 1)
            .with_runtime_admission(verifier, evidence, &substituted)
            .expect("admission");
        assert_eq!(
            server.zone_bootstrap(&bootstrap),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::PolicyDenial
            }
        );
        let (verifier, evidence) = authority.issue(sealed).expect("issued");
        let enroll = enroll_request(&substituted, SEALED_PEER_FINGERPRINT)
            .with_runtime_admission(verifier, evidence, &substituted)
            .expect("admission");
        assert_eq!(
            server.zone_enroll(&enroll),
            ZoneEnrollReply::Refused {
                reason: ZoneEnrollmentRefusal::PolicyDenial
            }
        );
        // The refusals consumed both admissions and moved no link state.
        assert_eq!(server.link_state(&zone(&["k1", "k0"])), None);
    }

    #[test]
    fn enrollment_commits_and_admits_the_peer_with_the_allocator_placement() {
        let mut server = server();
        let expected = enrolled_expectation();
        let bootstrap = bootstrap_request(&expected, 1)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert!(matches!(
            server.zone_bootstrap(&bootstrap),
            ZoneBootstrapReply::Admitted { .. }
        ));

        let enroll = enroll_request(&expected, [0x33; 32])
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_enroll(&enroll),
            ZoneEnrollReply::Enrolled {
                zone: expected.zone().clone(),
                generation: 1,
            }
        );
        assert_eq!(
            server.link_state(&zone(&["k1", "k0"])),
            Some(d2b_bus::session::ZoneLinkState::Ready)
        );
        assert!(
            server
                .audit_events()
                .all(|event| event.kind == ZoneRouteAuditEventKind::ZoneLinkSessionEstablished)
        );
    }

    #[test]
    fn an_enrollment_without_a_admitted_bootstrap_is_refused() {
        let mut server = server();
        let expected = enrolled_expectation();
        let enroll = enroll_request(&expected, [0x33; 32])
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_enroll(&enroll),
            ZoneEnrollReply::Refused {
                reason: ZoneEnrollmentRefusal::InvalidTransition
            }
        );
    }

    #[test]
    fn a_peer_that_does_not_match_the_sealed_enrollment_is_refused_without_downgrading() {
        let mut server = server();
        let expected = enrolled_expectation();
        admitted_bootstrap(&mut server, &expected);

        // The call presents a peer fingerprint other than the one the
        // allocator sealed, so the record never admits it.
        let mismatch = enroll_request(&expected, [0x99; 32])
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_enroll(&mismatch),
            ZoneEnrollReply::Refused {
                reason: ZoneEnrollmentRefusal::ZoneLinkEnrollmentKeyMismatch
            }
        );
        // The refusal falls back to the committed enrollment, never to a
        // bootstrap: the PSK is already burned and the sealed record stands.
        assert_eq!(
            server.link_state(&zone(&["k1", "k0"])),
            Some(d2b_bus::session::ZoneLinkState::EnrollmentCommitted)
        );

        let retry = enroll_request(&expected, SEALED_PEER_FINGERPRINT)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert!(matches!(
            server.zone_enroll(&retry),
            ZoneEnrollReply::Enrolled { generation: 1, .. }
        ));

        let bootstrap_again = bootstrap_request(&expected, 2)
            .with_admission(ZoneEnrollmentAdmission::for_test(expected.clone()));
        assert_eq!(
            server.zone_bootstrap(&bootstrap_again),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::InvalidTransition
            }
        );
    }

    #[test]
    fn a_revoked_authority_refuses_both_handlers() {
        let mut server = server();
        let expected = enrolled_expectation();
        let now = Arc::new(AtomicU64::new(1_700_000_000_400));
        let clock: Arc<dyn Fn() -> u64 + Send + Sync> = {
            let now = Arc::clone(&now);
            Arc::new(move || now.load(Ordering::Acquire))
        };
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock, 30_000).expect("valid authority");
        let (bootstrap_verifier, bootstrap_evidence) =
            authority.issue(expected.clone()).expect("issued");
        let (enroll_verifier, enroll_evidence) = authority.issue(expected.clone()).expect("issued");
        authority.revoke();

        let bootstrap = bootstrap_request(&expected, 1)
            .with_runtime_admission(bootstrap_verifier, bootstrap_evidence, &expected)
            .expect("admission");
        assert_eq!(
            server.zone_bootstrap(&bootstrap),
            ZoneBootstrapReply::Refused {
                reason: ZoneEnrollmentRefusal::PolicyDenial
            }
        );
        let enroll = enroll_request(&expected, [0x33; 32])
            .with_runtime_admission(enroll_verifier, enroll_evidence, &expected)
            .expect("admission");
        assert_eq!(
            server.zone_enroll(&enroll),
            ZoneEnrollReply::Refused {
                reason: ZoneEnrollmentRefusal::PolicyDenial
            }
        );
    }
}
