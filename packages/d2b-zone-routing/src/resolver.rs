//! Zone entrypoint resolution (`ADR046-routing-003`).
//!
//! The resolver sits directly above [`ZoneRouteEngine`]. It answers one
//! question: which sealed Zone is the *entrypoint* that owns a requested
//! target Zone, and may this call reach it? It resolves; it does not decide
//! routing. Once an entrypoint is selected the resolver hands the question
//! straight to the engine's [`ZoneRouteEngine::decide_route`] and reports the
//! engine's own typed outcome unchanged.
//!
//! Resolution is **policy, not address decoding**, exactly as in the v3
//! baseline realm entrypoint table this module adapts: the Zone path grammar
//! never encodes where a Zone's entrypoint lives, the sealed topology does.
//! Selection is a longest-suffix match over [`ZonePath`], so the nearest
//! sealed ancestor owns a target below it unless a more specific sealed Zone
//! matches first.
//!
//! The resolver is driven by exactly two inputs, both named by the work item:
//! the sealed `{ childZone, parentZone }` topology rows compiled from Nix, and
//! the authenticated admitted route projection already held by the engine.
//! There is no third source and no ambient default.
//!
//! What this module deliberately does not do: it performs no I/O, opens no
//! socket, verifies no signature, reads no store, and mints, holds, or
//! presents no authority. Nothing here accepts or returns a uid, gid, host
//! path, socket path, store path, transport endpoint, credential, or key
//! material. Every refusal carries one closed
//! [`ZoneRouteFailClosedReason`]; there is no permissive default and no silent
//! widening to a broader scope anywhere in this file.

use std::collections::{BTreeMap, BTreeSet};

use d2b_bus::session::{RouteAdmissionEvidence, RouteAdmissionVerifier};
use d2b_contracts_resource::v3::execution_policy::PrimitiveSpecError;
use d2b_contracts_zone_session::v3::zone_routing::{
    MAX_ZONE_PARENT_ENTRIES, ZONE_ROUTE_INITIAL_HOP_BUDGET, ZonePath, ZoneRouteAuditEventKind,
    ZoneRouteCapabilitySet, ZoneRouteFailClosedReason, ZoneRoutePath, ZoneTreeEdge,
};

use crate::engine::{
    ZoneRouteAdmission, ZoneRouteAdmissionExpectation, ZoneRouteDecision, ZoneRouteEngine,
    ZoneRouteRequest,
};

/// Render a type's `Debug` as its bare type name.
///
/// Zone paths, capability sets, and route metadata must never reach a log,
/// span, or metric through an incidental `Debug` on a container that holds
/// them, so every public type in this module opts out of a derived `Debug`.
/// The macro is module-private and adds no public item.
macro_rules! redacted_topology_debug {
    ($type_name:ident) => {
        impl ::core::fmt::Debug for $type_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(concat!(stringify!($type_name), "(redacted)"))
            }
        }
    };
}

/// The sealed Zone tree topology compiled from Nix `parentZone` declarations.
///
/// Each row is one `{ childZone, parentZone }` edge. The set is sealed at
/// construction: after [`SealedZoneTopology::seal`] returns there is no way to
/// add, replace, or remove a row, so a runtime peer can never grow the
/// topology the resolver matches against.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedZoneTopology {
    local_root: ZonePath,
    /// Every Zone the sealed topology declares, including the local root.
    zones: BTreeSet<ZonePath>,
}

impl SealedZoneTopology {
    /// Seal a topology rooted at `local_root` from its declared edges.
    ///
    /// Fails closed when the rows do not describe one well-formed tree under
    /// the local root:
    ///
    /// - an edge naming the local root as a child, since the root declares no
    ///   `parentZone` ([`PrimitiveSpecError::ConflictingFields`]);
    /// - two rows giving one child different parents, which is the ambiguous
    ///   topology case ([`PrimitiveSpecError::DuplicateEntry`]). A direct edge
    ///   determines its parent from its child path, so this is structurally
    ///   precluded by [`ZoneTreeEdge::new`]; the guard is defence in depth
    ///   rather than a reachable path today;
    /// - an edge whose parent is not itself the local root or a declared child,
    ///   so the row would attach a subtree outside the sealed scope
    ///   ([`PrimitiveSpecError::MissingRequiredField`]);
    /// - more rows than the frozen parent-entry ceiling
    ///   ([`PrimitiveSpecError::TooManyEntries`]).
    ///
    /// Each edge's parent/direct-child relationship was already proven by
    /// [`ZoneTreeEdge::new`], so it is not re-checked here.
    pub fn seal(
        local_root: ZonePath,
        edges: Vec<ZoneTreeEdge>,
    ) -> Result<Self, PrimitiveSpecError> {
        if edges.len() > MAX_ZONE_PARENT_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }

        let mut parent_of: BTreeMap<ZonePath, ZonePath> = BTreeMap::new();
        for edge in &edges {
            if edge.child() == &local_root {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
            match parent_of.get(edge.child()) {
                Some(existing) if existing == edge.parent() => {}
                Some(_) => return Err(PrimitiveSpecError::DuplicateEntry),
                None => {
                    parent_of.insert(edge.child().clone(), edge.parent().clone());
                }
            }
        }

        // Every declared parent must itself be reachable from the local root
        // through declared rows, so a row can never attach a subtree the
        // sealed topology does not otherwise contain.
        for parent in parent_of.values() {
            if parent == &local_root {
                continue;
            }
            if !parent_of.contains_key(parent) {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
        }

        let mut zones = BTreeSet::new();
        zones.insert(local_root.clone());
        for child in parent_of.keys() {
            if !child.is_descendant_of(&local_root) {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
            zones.insert(child.clone());
        }

        Ok(Self { local_root, zones })
    }

    /// Borrow the local root Zone path.
    pub const fn local_root(&self) -> &ZonePath {
        &self.local_root
    }

    /// Number of sealed Zones, including the local root.
    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    /// The nearest sealed Zone at or above `target` by longest-suffix match.
    ///
    /// Labels are most specific first, so progressively dropping leading
    /// labels tests successively shorter suffixes; the first hit is therefore
    /// the longest match. Returns `None` when no sealed Zone matches, which is
    /// the unknown-topology case.
    fn longest_suffix_match(&self, target: &ZonePath) -> Option<&ZonePath> {
        let labels = target.labels();
        for start in 0..labels.len() {
            // A non-empty sub-slice of a valid Zone path is itself a valid
            // Zone path, so a suffix can only fail to build if the slice is
            // empty, which the loop bound excludes.
            let Ok(suffix) = ZonePath::new(labels[start..].to_vec()) else {
                continue;
            };
            if let Some(zone) = self.zones.get(&suffix) {
                return Some(zone);
            }
        }
        None
    }
}

redacted_topology_debug!(SealedZoneTopology);

/// One entrypoint question posed to the resolver.
///
/// A remote entrypoint carries a verified runtime-issued admission. There is
/// no caller-populated time, policy, connectivity, capability, or
/// authentication claim on this request.
pub struct ZoneEntrypointRequest {
    /// The Zone the call targets.
    target_zone: ZonePath,
    /// Hops still available to this call.
    remaining_hops: u32,
    /// The verified runtime-issued route admission.
    admission: Option<ZoneRouteAdmission>,
}

impl ZoneEntrypointRequest {
    /// A request with no runtime admission.
    ///
    /// Local-root resolution remains representable; remote resolution refuses
    /// until a verified admission is attached.
    pub const fn new(target_zone: ZonePath) -> Self {
        Self {
            target_zone,
            remaining_hops: ZONE_ROUTE_INITIAL_HOP_BUDGET,
            admission: None,
        }
    }

    /// Build an entrypoint request by consuming one runtime-issued admission.
    pub fn from_runtime_admission(
        target_zone: ZonePath,
        remaining_hops: u32,
        verifier: RouteAdmissionVerifier,
        evidence: RouteAdmissionEvidence,
        expected: &ZoneRouteAdmissionExpectation,
    ) -> Result<Self, ZoneRouteFailClosedReason> {
        let admission = ZoneRouteAdmission::verify(verifier, evidence, expected)?;
        Ok(Self::new(target_zone)
            .with_remaining_hops(remaining_hops)
            .with_admission(admission))
    }

    /// Attach a verified runtime-issued admission.
    pub fn with_admission(mut self, admission: ZoneRouteAdmission) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Set a bounded remaining hop budget.
    pub const fn with_remaining_hops(mut self, remaining_hops: u32) -> Self {
        self.remaining_hops = remaining_hops;
        self
    }

    /// Borrow the target Zone path.
    pub const fn target_zone(&self) -> &ZonePath {
        &self.target_zone
    }

    /// Return the remaining hop budget.
    pub const fn remaining_hops(&self) -> u32 {
        self.remaining_hops
    }

    /// Borrow the verified admission, when one was attached.
    pub const fn admission(&self) -> Option<&ZoneRouteAdmission> {
        self.admission.as_ref()
    }
}

redacted_topology_debug!(ZoneEntrypointRequest);

/// The resolver's answer to one entrypoint question.
#[derive(Clone, PartialEq, Eq)]
pub enum ZoneEntrypointResolution {
    /// The target resolves to an entrypoint the caller may reach.
    Resolved {
        /// The sealed Zone that owns the target subtree. It is the target
        /// itself when the target is sealed, and its nearest sealed ancestor
        /// otherwise.
        entrypoint_zone: ZonePath,
        /// Immutable route metadata from the engine; it carries no transport,
        /// endpoint, or credential.
        path: ZoneRoutePath,
        /// The capability ceiling surviving every advertised hop, or `None`
        /// only for exact local-root dispatch.
        effective_capabilities: Option<ZoneRouteCapabilitySet>,
        /// Hops left after paying for this path.
        remaining_hops_after: u32,
    },
    /// The target could not be resolved, or the resolved entrypoint may not be
    /// reached.
    Refused {
        /// The closed refusal reason.
        reason: ZoneRouteFailClosedReason,
    },
}

impl ZoneEntrypointResolution {
    /// The audit event kind this outcome emits.
    ///
    /// Resolution is one leg of a route decision, so it reuses the route
    /// decision event kinds rather than introducing a parallel taxonomy.
    pub const fn audit_event(&self) -> ZoneRouteAuditEventKind {
        match self {
            Self::Resolved { .. } => ZoneRouteAuditEventKind::ZoneRouteAllowed,
            Self::Refused { .. } => ZoneRouteAuditEventKind::ZoneRouteDenied,
        }
    }

    /// The refusal reason, when the outcome is a refusal.
    pub const fn denial_reason(&self) -> Option<ZoneRouteFailClosedReason> {
        match self {
            Self::Resolved { .. } => None,
            Self::Refused { reason } => Some(*reason),
        }
    }
}

redacted_topology_debug!(ZoneEntrypointResolution);

/// Resolves a target Zone to its sealed entrypoint, then defers the route
/// decision to [`ZoneRouteEngine`].
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneEntrypointResolver {
    topology: SealedZoneTopology,
}

impl ZoneEntrypointResolver {
    /// Build a resolver over a sealed topology.
    pub const fn new(topology: SealedZoneTopology) -> Self {
        Self { topology }
    }

    /// Borrow the sealed topology.
    pub const fn topology(&self) -> &SealedZoneTopology {
        &self.topology
    }

    /// Resolve `request` against the sealed topology and `engine`'s admitted
    /// route projection.
    ///
    /// The order is: engine agreement, local scope, the longest-suffix match,
    /// the unknown-Zone guard below the local root, admission presence, and
    /// finally the engine's own route decision. Every stage before the engine
    /// refuses with a closed reason; the engine's refusal is reported
    /// unchanged.
    pub fn resolve(
        &self,
        engine: &ZoneRouteEngine,
        request: &ZoneEntrypointRequest,
    ) -> ZoneEntrypointResolution {
        self.resolve_parts(
            engine,
            &request.target_zone,
            request.remaining_hops,
            request.admission.as_ref(),
        )
    }

    /// Resolve a target using an admission borrowed from a larger projection.
    pub fn resolve_with_admission(
        &self,
        engine: &ZoneRouteEngine,
        target_zone: &ZonePath,
        remaining_hops: u32,
        admission: &ZoneRouteAdmission,
    ) -> ZoneEntrypointResolution {
        self.resolve_parts(engine, target_zone, remaining_hops, Some(admission))
    }

    fn resolve_parts(
        &self,
        engine: &ZoneRouteEngine,
        target_zone: &ZonePath,
        remaining_hops: u32,
        admission: Option<&ZoneRouteAdmission>,
    ) -> ZoneEntrypointResolution {
        let refused = |reason| ZoneEntrypointResolution::Refused { reason };
        let local_root = self.topology.local_root();

        // The sealed topology and the projection engine must describe the same
        // local root. A disagreement means the resolver was handed a
        // projection for a different Zone runtime, which is out of scope.
        if engine.local_root() != local_root {
            return refused(ZoneRouteFailClosedReason::PolicyDenial);
        }

        // Scope violation: a target that is neither the local root nor below
        // it is outside this runtime's authority entirely.
        if target_zone != local_root && !target_zone.is_descendant_of(local_root) {
            return refused(ZoneRouteFailClosedReason::PolicyDenial);
        }

        let Some(entrypoint_zone) = self.topology.longest_suffix_match(target_zone) else {
            return refused(ZoneRouteFailClosedReason::UnknownParent);
        };
        let entrypoint_zone = entrypoint_zone.clone();

        let exact = entrypoint_zone == *target_zone;

        // The local root is the suffix of every in-scope path, so letting it
        // absorb an unmatched target would make the resolver unconditionally
        // permissive and defeat the work item's "fail closed on unknown
        // topology" requirement. A non-root sealed Zone legitimately covers
        // descendants the local runtime cannot enumerate; the local root does
        // not, because its own children are all sealed.
        if !exact && &entrypoint_zone == local_root {
            return refused(ZoneRouteFailClosedReason::UnknownParent);
        }

        // A remote entrypoint is reachable only with a verified runtime-issued
        // admission. Local dispatch consults no ZoneLink projection.
        if &entrypoint_zone != local_root && admission.is_none() {
            return refused(ZoneRouteFailClosedReason::PolicyDenial);
        }

        let decision = match admission {
            Some(admission) => engine.decide_route_via_entrypoint(
                local_root,
                &entrypoint_zone,
                target_zone,
                remaining_hops,
                admission,
            ),
            None => {
                let route_request =
                    ZoneRouteRequest::new(local_root.clone(), entrypoint_zone.clone())
                        .with_remaining_hops(remaining_hops);
                engine.decide_route(&route_request)
            }
        };

        match decision {
            ZoneRouteDecision::Allowed {
                path,
                effective_capabilities,
                remaining_hops_after,
            } => ZoneEntrypointResolution::Resolved {
                entrypoint_zone,
                path,
                effective_capabilities,
                remaining_hops_after,
            },
            ZoneRouteDecision::Denied { reason } => refused(reason),
        }
    }
}

redacted_topology_debug!(ZoneEntrypointResolver);

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{ResourceUid, ZoneRevision, identity::ReconnectGeneration};
    use d2b_contracts_zone_session::v3::{
        component_session::{OperationClass, OperationId},
        zone_routing::{
            ZONE_ROUTING_SCHEMA_VERSION, ZoneDescendantRoute, ZoneLabelId,
            ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation, ZoneLinkRouteAdvertisement,
            ZoneRouteCapability, ZoneRouteId, ZoneRouteKeyRole, ZoneRouteSignature,
            ZoneRouteSignatureAlgorithm, ZoneRouteSignatureRef, ZoneSigningKeyFingerprint,
        },
    };

    use crate::engine::{
        ZoneAdvertisementAdmission, ZoneRouteAdmission, ZoneRouteAdmissionExpectation,
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
        admission_for_target(
            &zone(&["k2", "k1", "k0"]),
            capability,
            issued_at,
            expires_at,
        )
    }

    fn admission_for_target(
        target: &ZonePath,
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
            OperationClass::Invoke,
            ZoneRouteCapability::parse(capability).expect("valid capability"),
            ZoneRevision::new(9),
        )
        .expect("valid route admission expectation")
        .for_zones(zone(&["k0"]), target.clone());
        ZoneRouteAdmission::for_test(expectation, issued_at, expires_at)
    }

    /// Sealed topology: root k0, child k1, grandchild k2 under k1.
    fn sealed() -> SealedZoneTopology {
        SealedZoneTopology::seal(
            zone(&["k0"]),
            vec![
                edge(&["k0"], &["k1", "k0"]),
                edge(&["k1", "k0"], &["k2", "k1", "k0"]),
            ],
        )
        .expect("well formed topology")
    }

    /// Engine rooted at k0 with an admitted route to k2 through k1.
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
        let outcome = engine.admit_advertisement(&advertisement, &allocation, 1_500);
        assert!(matches!(
            outcome,
            ZoneAdvertisementAdmission::Accepted { .. }
        ));
        engine
    }

    fn allowed_request(target: ZonePath) -> ZoneEntrypointRequest {
        ZoneEntrypointRequest::new(target.clone())
            .with_admission(admission_for_target(&target, "get", 1_500, 4_000))
    }

    // -- sealing ----------------------------------------------------------

    #[test]
    fn a_target_can_only_match_one_sealed_entrypoint() {
        // Ambiguity would mean two sealed Zones both owning one target. Two
        // sealed Zones that both suffix-match a target are necessarily nested,
        // and longest-suffix match picks the most specific one deterministically,
        // so the more general candidate can never also be selected.
        let topology = sealed();
        let target = zone(&["deep", "k2", "k1", "k0"]);
        let selected = topology
            .longest_suffix_match(&target)
            .expect("a sealed ancestor owns the target")
            .clone();
        let also_matching: Vec<ZonePath> = topology
            .zones
            .iter()
            .filter(|candidate| target.is_descendant_of(candidate) || &&target == candidate)
            .cloned()
            .collect();
        // Three sealed Zones match by suffix; exactly the deepest is selected.
        assert_eq!(also_matching.len(), 3);
        assert_eq!(selected, zone(&["k2", "k1", "k0"]));
        for candidate in also_matching {
            assert!(candidate == selected || selected.is_descendant_of(&candidate));
        }
    }

    #[test]
    fn sealing_accepts_a_repeated_identical_row_without_growing_the_topology() {
        let topology = SealedZoneTopology::seal(
            zone(&["k0"]),
            vec![edge(&["k0"], &["k1", "k0"]), edge(&["k0"], &["k1", "k0"])],
        )
        .expect("identical rows are idempotent");
        assert_eq!(topology.zone_count(), 2);
    }

    #[test]
    fn sealing_rejects_the_local_root_as_a_child() {
        let error =
            SealedZoneTopology::seal(zone(&["k1", "k0"]), vec![edge(&["k0"], &["k1", "k0"])])
                .expect_err("the local root declares no parent");
        assert_eq!(error, PrimitiveSpecError::ConflictingFields);
    }

    #[test]
    fn sealing_rejects_a_subtree_attached_outside_the_sealed_scope() {
        let error = SealedZoneTopology::seal(
            zone(&["k0"]),
            // k9.k0 is never declared as a child, so k1.k9.k0 would attach an
            // unknown subtree.
            vec![edge(&["k9", "k0"], &["k1", "k9", "k0"])],
        )
        .expect_err("an undeclared parent fails closed");
        assert_eq!(error, PrimitiveSpecError::MissingRequiredField);
    }

    // -- longest-suffix match vectors -------------------------------------

    #[test]
    fn an_exact_sealed_zone_matches_itself() {
        let topology = sealed();
        assert_eq!(
            topology.longest_suffix_match(&zone(&["k1", "k0"])),
            Some(&zone(&["k1", "k0"]))
        );
    }

    #[test]
    fn a_descendant_matches_its_nearest_sealed_ancestor_not_the_root() {
        let topology = sealed();
        // deep.k2.k1.k0 is not sealed; the longest suffix is k2.k1.k0, and the
        // shorter suffixes k1.k0 and k0 must not win.
        assert_eq!(
            topology.longest_suffix_match(&zone(&["deep", "k2", "k1", "k0"])),
            Some(&zone(&["k2", "k1", "k0"]))
        );
    }

    #[test]
    fn a_descendant_of_an_unsealed_sibling_falls_back_to_the_sealed_parent() {
        let topology = sealed();
        assert_eq!(
            topology.longest_suffix_match(&zone(&["billing", "k1", "k0"])),
            Some(&zone(&["k1", "k0"]))
        );
    }

    #[test]
    fn a_foreign_root_matches_nothing() {
        let topology = sealed();
        assert_eq!(topology.longest_suffix_match(&zone(&["other-root"])), None);
    }

    // -- resolution -------------------------------------------------------

    #[test]
    fn request_defaults_refuse_before_any_topology_is_consulted() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]));
        assert_eq!(request.remaining_hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);
        let resolution = resolver.resolve(&engine, &request);
        // Missing runtime admission defaults to refusing.
        assert_eq!(
            resolution.denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
        assert_eq!(
            resolution.audit_event(),
            ZoneRouteAuditEventKind::ZoneRouteDenied
        );
    }

    #[test]
    fn the_local_root_resolves_to_itself_with_a_zero_hop_path() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        // Local dispatch consults no projection, so it needs no authenticated
        // projection input.
        let request = ZoneEntrypointRequest::new(zone(&["k0"]));
        let ZoneEntrypointResolution::Resolved {
            entrypoint_zone,
            path,
            effective_capabilities,
            remaining_hops_after,
        } = resolver.resolve(&engine, &request)
        else {
            panic!("expected a resolved local entrypoint");
        };
        assert_eq!(entrypoint_zone, zone(&["k0"]));
        assert_eq!(path.hop_count(), 0);
        assert!(effective_capabilities.is_none());
        assert_eq!(remaining_hops_after, ZONE_ROUTE_INITIAL_HOP_BUDGET);
    }

    #[test]
    fn resolve_then_decide_returns_the_engine_path_for_a_sealed_descendant() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = allowed_request(zone(&["k2", "k1", "k0"]));
        let ZoneEntrypointResolution::Resolved {
            entrypoint_zone,
            path,
            effective_capabilities,
            remaining_hops_after,
        } = resolver.resolve(&engine, &request)
        else {
            panic!("expected a resolved remote entrypoint");
        };
        assert_eq!(entrypoint_zone, zone(&["k2", "k1", "k0"]));
        assert_eq!(path.source_zone(), &zone(&["k0"]));
        assert_eq!(path.target_zone(), &zone(&["k2", "k1", "k0"]));
        assert_eq!(path.hop_count(), 2);
        assert_eq!(remaining_hops_after, ZONE_ROUTE_INITIAL_HOP_BUDGET - 2);
        assert_eq!(effective_capabilities, Some(caps(&["get", "list"])));
    }

    #[test]
    fn an_unsealed_descendant_resolves_to_its_sealed_ancestor_entrypoint() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        // deep.k2.k1.k0 has no sealed row and no projection of its own; the
        // sealed k2.k1.k0 owns it and is the entrypoint the engine routes to.
        let request = allowed_request(zone(&["deep", "k2", "k1", "k0"]));
        let ZoneEntrypointResolution::Resolved {
            entrypoint_zone,
            path,
            ..
        } = resolver.resolve(&engine, &request)
        else {
            panic!("expected the sealed ancestor to own the target");
        };
        assert_eq!(entrypoint_zone, zone(&["k2", "k1", "k0"]));
        assert_eq!(path.target_zone(), &zone(&["k2", "k1", "k0"]));
    }

    #[test]
    fn an_unknown_zone_directly_below_the_local_root_fails_closed() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = allowed_request(zone(&["unknown", "k0"]));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_target_outside_the_local_root_subtree_is_a_scope_violation() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = allowed_request(zone(&["k1", "other-root"]));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn a_projection_for_a_different_local_root_is_a_scope_violation() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = ZoneRouteEngine::new(zone(&["other-root"]));
        let request = allowed_request(zone(&["k1", "k0"]));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn an_unauthenticated_route_projection_refuses_a_remote_entrypoint() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
    }

    #[test]
    fn an_absent_projection_for_a_sealed_zone_fails_closed() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        // A sealed topology with no admitted advertisement at all.
        let engine = ZoneRouteEngine::new(zone(&["k0"]));
        let request = allowed_request(zone(&["k2", "k1", "k0"]));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_stale_projection_fails_closed_at_the_engine() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        // The seeded advertisement expires at 4000.
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]))
            .with_admission(admission("get", 9_000, 10_000));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent)
        );
    }

    #[test]
    fn a_missing_capability_at_the_entrypoint_is_reported_unchanged() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let engine = seeded_engine();
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]))
            .with_admission(admission("watch", 1_500, 4_000));
        assert_eq!(
            resolver.resolve(&engine, &request).denial_reason(),
            Some(ZoneRouteFailClosedReason::MissingCapability)
        );
    }

    #[test]
    fn public_debug_renders_no_zone_path() {
        let resolver = ZoneEntrypointResolver::new(sealed());
        let request = ZoneEntrypointRequest::new(zone(&["k2", "k1", "k0"]));
        for rendered in [
            format!("{resolver:?}"),
            format!("{:?}", resolver.topology()),
            format!("{request:?}"),
            format!(
                "{:?}",
                ZoneEntrypointResolution::Refused {
                    reason: ZoneRouteFailClosedReason::UnknownParent
                }
            ),
        ] {
            assert!(rendered.contains("redacted"), "rendered: {rendered}");
            assert!(!rendered.contains("k0"), "rendered: {rendered}");
        }
    }
}
