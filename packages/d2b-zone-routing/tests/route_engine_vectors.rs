//! External golden vectors for the Zone route decision engine
//! (`ADR046-routing-006`).
//!
//! This suite exercises `d2b_zone_routing::engine` strictly through its public
//! API, from outside the crate, so nothing here can reach a private field or a
//! private helper. It is deliberately a different layer from the engine's
//! in-file unit tests: every case below is a row in a named vector table, and
//! each table is one of the classes the work item names - advertisement
//! admission, nearest-common-ancestor walks, loop and multi-parent detection,
//! the capability ceiling, the replay window, the K0/K1/K2 topology, and the
//! hop-count boundary.
//!
//! A vector row states an input and the exact typed outcome expected of it.
//! Refusals are compared against a single closed
//! [`ZoneRouteFailClosedReason`], never against a message, so a reason
//! renaming or a reordering of the engine's refusal stages is caught here
//! rather than absorbed. No row asserts wall-clock behaviour; timing lives in
//! the companion `route_decision` benchmark.

use d2b_contracts::v3::zone_routing::{
    MAX_ZONE_ROUTE_PATH_HOPS, ZONE_ROUTE_INITIAL_HOP_BUDGET, ZONE_ROUTING_SCHEMA_VERSION,
    ZoneDescendantRoute, ZoneLabelId, ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation,
    ZoneLinkRouteAdvertisement, ZoneLinkRouteWithdrawal, ZonePath, ZoneRouteAuditEventKind,
    ZoneRouteCapability, ZoneRouteCapabilitySet, ZoneRouteFailClosedReason, ZoneRouteHopDirection,
    ZoneRouteId, ZoneRouteKeyRole, ZoneRouteSignature, ZoneRouteSignatureAlgorithm,
    ZoneRouteSignatureRef, ZoneSigningKeyFingerprint, ZoneTreeEdge,
};
use d2b_zone_routing::engine::{
    ZoneAdvertisementAdmission, ZoneRelayAdmission, ZoneRelayRequest, ZoneRouteDecision,
    ZoneRouteEngine, ZoneRouteRequest, ZoneWithdrawalAdmission,
};

// ---------------------------------------------------------------------------
// Vector fixture construction
// ---------------------------------------------------------------------------

/// The instant every vector decides at, inside both seeded advertisement
/// windows.
const NOW: u64 = 1_500;

/// Build a Zone path from leaf-first labels, matching `ZonePath`'s own order.
fn zone(labels: &[&str]) -> ZonePath {
    ZonePath::new(
        labels
            .iter()
            .map(|label| ZoneLabelId::parse(*label).expect("valid zone label"))
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

fn route_id(value: &str) -> ZoneRouteId {
    ZoneRouteId::parse(value).expect("valid route id")
}

fn generation(value: &str) -> ZoneLinkControllerGeneration {
    ZoneLinkControllerGeneration::parse(value).expect("valid controller generation")
}

/// A signature record whose reference is the only field a vector varies.
///
/// The engine treats this as opaque replay bookkeeping; it verifies nothing,
/// and neither does this suite. The fingerprint is a fixed synthetic locator,
/// not key material.
fn signature(signature_ref: &str) -> ZoneRouteSignature {
    ZoneRouteSignature::new(
        ZoneRouteSignatureAlgorithm::Ed25519Blake3,
        ZoneRouteKeyRole::ZoneControllerRouting,
        ZoneSigningKeyFingerprint::parse(format!("sha256.{}", "c".repeat(64)))
            .expect("valid fingerprint"),
        ZoneRouteSignatureRef::parse(signature_ref).expect("valid signature ref"),
    )
}

/// One advertisement under construction, so a vector row can vary exactly the
/// field it is about and inherit a valid value for everything else.
struct AdvertVector {
    parent: ZonePath,
    child: ZonePath,
    generation: ZoneLinkControllerGeneration,
    routes: Vec<ZoneDescendantRoute>,
    issued_at: u64,
    expires_at: u64,
    signature_ref: String,
}

impl AdvertVector {
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
    generation_value: &str,
    prefixes: Vec<ZonePath>,
    max_routes: u32,
    allowed: &[&str],
) -> ZoneLinkNamespaceAllocation {
    ZoneLinkNamespaceAllocation::new(
        ZoneTreeEdge::new(parent, child).expect("direct child edge"),
        generation(generation_value),
        prefixes,
        max_routes,
        caps(allowed),
    )
    .expect("valid namespace allocation")
}

fn k0() -> ZonePath {
    zone(&["k0"])
}

fn k1() -> ZonePath {
    zone(&["k1", "k0"])
}

fn k2() -> ZonePath {
    zone(&["k2", "k1", "k0"])
}

fn k3() -> ZonePath {
    zone(&["k3", "k2", "k1", "k0"])
}

/// The K0/K1/K2 advertisement: K1 advertises reachability to K2 over the K0-K1
/// edge.
fn k1_advertisement() -> (ZoneLinkRouteAdvertisement, ZoneLinkNamespaceAllocation) {
    let advert = AdvertVector::new(k0(), k1())
        .route("route-k2", k2(), "k2", &["get", "list"])
        .build();
    let alloc = allocation(
        k0(),
        k1(),
        "gen-1",
        vec![k1()],
        8,
        &["get", "list", "watch"],
    );
    (advert, alloc)
}

/// The K2 extension: K2 advertises reachability to K3 over the K1-K2 edge with
/// a narrower capability set, so the ceiling narrows down the tree.
fn k2_advertisement() -> (ZoneLinkRouteAdvertisement, ZoneLinkNamespaceAllocation) {
    let advert = AdvertVector::new(k1(), k2())
        .route("route-k3", k3(), "k3", &["get"])
        .window(1_100, 4_000)
        .signature_ref("sigref-2")
        .generation("gen-2")
        .build();
    let alloc = allocation(
        k1(),
        k2(),
        "gen-2",
        vec![k2()],
        8,
        &["get", "list", "watch"],
    );
    (advert, alloc)
}

/// An engine holding the two-level K0/K1/K2 topology.
fn k0_k1_k2_engine() -> ZoneRouteEngine {
    let mut engine = ZoneRouteEngine::new(k0());
    let (advert, alloc) = k1_advertisement();
    assert_accepted(
        engine.admit_advertisement(&advert, &alloc, NOW),
        &["route-k2"],
    );
    engine
}

/// An engine holding the three-level K0/K1/K2/K3 topology.
fn k0_k1_k2_k3_engine() -> ZoneRouteEngine {
    let mut engine = k0_k1_k2_engine();
    let (advert, alloc) = k2_advertisement();
    assert_accepted(
        engine.admit_advertisement(&advert, &alloc, NOW),
        &["route-k3"],
    );
    engine
}

/// A request whose connectivity and authorization inputs are both satisfied,
/// so the refusing stage under test is the one the vector is about.
fn routable(source: ZonePath, target: ZonePath) -> ZoneRouteRequest {
    let mut request = ZoneRouteRequest::new(source, target, NOW);
    request.policy_allows = true;
    request.zone_link_connected = true;
    request
}

fn assert_accepted(outcome: ZoneAdvertisementAdmission, expected_routes: &[&str]) {
    let ZoneAdvertisementAdmission::Accepted { accepted_routes } = outcome else {
        panic!("expected an accepted advertisement, got {outcome:?}");
    };
    let expected = expected_routes
        .iter()
        .map(|id| route_id(id))
        .collect::<Vec<_>>();
    assert_eq!(accepted_routes, expected);
}

// ---------------------------------------------------------------------------
// Class: advertisement admission
// ---------------------------------------------------------------------------

/// One advertisement-admission vector.
struct AdmissionVector {
    name: &'static str,
    advertisement: ZoneLinkRouteAdvertisement,
    allocation: ZoneLinkNamespaceAllocation,
    received_at: u64,
    /// `None` means the row must be admitted.
    expected: Option<ZoneRouteFailClosedReason>,
}

#[test]
fn advertisement_admission_vectors() {
    let vectors = vec![
        AdmissionVector {
            name: "well-formed child advertisement is admitted",
            advertisement: k1_advertisement().0,
            allocation: k1_advertisement().1,
            received_at: NOW,
            expected: None,
        },
        AdmissionVector {
            name: "advertisement received before it was issued is malformed",
            advertisement: k1_advertisement().0,
            allocation: k1_advertisement().1,
            received_at: 900,
            expected: Some(ZoneRouteFailClosedReason::MalformedAdvert),
        },
        AdmissionVector {
            name: "advertisement received after its window closed is expired",
            advertisement: k1_advertisement().0,
            allocation: k1_advertisement().1,
            received_at: 4_000,
            expected: Some(ZoneRouteFailClosedReason::Expired),
        },
        AdmissionVector {
            name: "advertisement whose parent is not projected is refused",
            advertisement: AdvertVector::new(k1(), k2())
                .route("route-k3", k3(), "k3", &["get"])
                .build(),
            allocation: allocation(k1(), k2(), "gen-1", vec![k2()], 8, &["get"]),
            received_at: NOW,
            expected: Some(ZoneRouteFailClosedReason::UnknownParent),
        },
        AdmissionVector {
            name: "route outside the allocated prefix violates the namespace",
            advertisement: AdvertVector::new(k0(), k1())
                .route("route-k2", k2(), "k2", &["get"])
                .build(),
            // The allocation delegates only a sibling subtree, so K2 is a
            // structurally valid descendant that this advertiser was never
            // given.
            allocation: allocation(
                k0(),
                k1(),
                "gen-1",
                vec![zone(&["k4", "k1", "k0"])],
                8,
                &["get"],
            ),
            received_at: NOW,
            expected: Some(ZoneRouteFailClosedReason::NamespaceViolation),
        },
        AdmissionVector {
            name: "route capability above the allocated ceiling violates the namespace",
            advertisement: AdvertVector::new(k0(), k1())
                .route("route-k2", k2(), "k2", &["get", "delete"])
                .build(),
            allocation: allocation(k0(), k1(), "gen-1", vec![k1()], 8, &["get"]),
            received_at: NOW,
            expected: Some(ZoneRouteFailClosedReason::NamespaceViolation),
        },
        AdmissionVector {
            name: "more routes than the allocation permits violates the namespace",
            advertisement: AdvertVector::new(k0(), k1())
                .route("route-k2", k2(), "k2", &["get"])
                .route("route-k3", k3(), "k2", &["get"])
                .build(),
            allocation: allocation(k0(), k1(), "gen-1", vec![k1()], 1, &["get"]),
            received_at: NOW,
            expected: Some(ZoneRouteFailClosedReason::NamespaceViolation),
        },
        AdmissionVector {
            name: "allocation bound to another generation is refused",
            advertisement: k1_advertisement().0,
            allocation: allocation(k0(), k1(), "gen-9", vec![k1()], 8, &["get", "list"]),
            received_at: NOW,
            expected: Some(ZoneRouteFailClosedReason::NamespaceViolation),
        },
    ];

    for vector in vectors {
        let mut engine = ZoneRouteEngine::new(k0());
        let outcome = engine.admit_advertisement(
            &vector.advertisement,
            &vector.allocation,
            vector.received_at,
        );
        assert_eq!(
            outcome.denial_reason(),
            vector.expected,
            "advertisement vector: {}",
            vector.name
        );
        let expected_event = if vector.expected.is_none() {
            ZoneRouteAuditEventKind::ZoneAdvertisementAccepted
        } else {
            ZoneRouteAuditEventKind::ZoneAdvertisementDenied
        };
        assert_eq!(
            outcome.audit_event(),
            expected_event,
            "advertisement audit event: {}",
            vector.name
        );
        // A refused advertisement must leave the projection untouched.
        if vector.expected.is_some() {
            assert!(
                engine.route_inventory().is_empty(),
                "refused advertisement installed a route: {}",
                vector.name
            );
        }
    }
}

/// A Zone cannot even construct an advertisement for a sibling, its own
/// parent, or itself: the wire contract's own constructor rejects the shape
/// before the engine sees it.
///
/// This matters for reading the engine's refusal set. Its
/// `SiblingOrParentRouteAdvert` reason for advertisements is a defence for the
/// deserialization path, not a state a caller holding a validated
/// advertisement can reach, so no vector above can produce it. The withdrawal
/// vectors below do reach that reason, because a withdrawal names an
/// already-installed route rather than a structural descendant.
#[test]
fn sibling_and_parent_route_advertisement_vectors() {
    let vectors: Vec<(&str, ZonePath, &str)> = vec![
        ("the advertiser's own parent", k0(), "k0"),
        ("a sibling of the advertiser", zone(&["k7", "k0"]), "k7"),
        ("the advertiser itself", k1(), "k1"),
        ("a Zone in a disjoint tree", zone(&["z1", "z0"]), "z1"),
    ];

    for (name, descendant, next_hop) in vectors {
        let built = ZoneLinkRouteAdvertisement::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            k1(),
            ZoneTreeEdge::new(k0(), k1()).expect("direct child edge"),
            generation("gen-1"),
            vec![ZoneDescendantRoute::new(
                route_id("route-bad"),
                descendant,
                ZoneLabelId::parse(next_hop).expect("valid label"),
                caps(&["get"]),
            )],
            1_000,
            4_000,
            signature("sigref-bad"),
        );
        assert!(
            built.is_err(),
            "non-descendant advertisement was accepted: {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Class: nearest-common-ancestor walks
// ---------------------------------------------------------------------------

/// One nearest-common-ancestor vector over the K0/K1/K2/K3 topology.
struct NcaVector {
    name: &'static str,
    source: ZonePath,
    target: ZonePath,
    expected_ancestor: ZonePath,
    expected_up_hops: usize,
    expected_down_hops: usize,
}

#[test]
fn nearest_common_ancestor_vectors() {
    let engine = k0_k1_k2_k3_engine();
    let vectors = vec![
        NcaVector {
            name: "root to itself is a zero-hop path",
            source: k0(),
            target: k0(),
            expected_ancestor: k0(),
            expected_up_hops: 0,
            expected_down_hops: 0,
        },
        NcaVector {
            name: "root down to its direct child",
            source: k0(),
            target: k1(),
            expected_ancestor: k0(),
            expected_up_hops: 0,
            expected_down_hops: 1,
        },
        NcaVector {
            name: "root down to a grandchild",
            source: k0(),
            target: k2(),
            expected_ancestor: k0(),
            expected_up_hops: 0,
            expected_down_hops: 2,
        },
        NcaVector {
            name: "root down to a great-grandchild",
            source: k0(),
            target: k3(),
            expected_ancestor: k0(),
            expected_up_hops: 0,
            expected_down_hops: 3,
        },
        NcaVector {
            name: "a descendant walks up to the root",
            source: k3(),
            target: k0(),
            expected_ancestor: k0(),
            expected_up_hops: 3,
            expected_down_hops: 0,
        },
        NcaVector {
            name: "the ancestor of a straight-line pair is the higher endpoint",
            source: k1(),
            target: k3(),
            expected_ancestor: k1(),
            expected_up_hops: 0,
            expected_down_hops: 2,
        },
        NcaVector {
            name: "an intermediate Zone reaches its own child directly",
            source: k2(),
            target: k3(),
            expected_ancestor: k2(),
            expected_up_hops: 0,
            expected_down_hops: 1,
        },
    ];

    for vector in vectors {
        let mut request = routable(vector.source.clone(), vector.target.clone());
        request.remaining_hops = ZONE_ROUTE_INITIAL_HOP_BUDGET;
        let decision = engine.decide_route(&request);
        let ZoneRouteDecision::Allowed {
            path,
            remaining_hops_after,
            ..
        } = decision
        else {
            panic!("NCA vector {} was refused: {decision:?}", vector.name);
        };
        assert_eq!(
            path.nearest_common_ancestor(),
            &vector.expected_ancestor,
            "NCA vector: {}",
            vector.name
        );
        let up = path
            .hops()
            .iter()
            .filter(|hop| hop.direction() == ZoneRouteHopDirection::UpToParent)
            .count();
        let down = path
            .hops()
            .iter()
            .filter(|hop| hop.direction() == ZoneRouteHopDirection::DownToChild)
            .count();
        assert_eq!(up, vector.expected_up_hops, "up hops: {}", vector.name);
        assert_eq!(
            down, vector.expected_down_hops,
            "down hops: {}",
            vector.name
        );
        assert_eq!(
            path.hop_count(),
            vector.expected_up_hops + vector.expected_down_hops,
            "hop count: {}",
            vector.name
        );
        assert_eq!(
            remaining_hops_after,
            ZONE_ROUTE_INITIAL_HOP_BUDGET - path.hop_count() as u32,
            "budget accounting: {}",
            vector.name
        );
        assert_eq!(
            decision_event(&engine, &vector.source, &vector.target),
            ZoneRouteAuditEventKind::ZoneRouteAllowed,
            "audit event: {}",
            vector.name
        );
    }
}

fn decision_event(
    engine: &ZoneRouteEngine,
    source: &ZonePath,
    target: &ZonePath,
) -> ZoneRouteAuditEventKind {
    engine
        .decide_route(&routable(source.clone(), target.clone()))
        .audit_event()
}

/// Zones the projection has never heard of, and Zones in a disjoint tree, are
/// both refused rather than walked.
#[test]
fn unknown_and_disjoint_zone_vectors() {
    let engine = k0_k1_k2_k3_engine();
    let vectors: Vec<(&str, ZonePath, ZonePath)> = vec![
        (
            "unknown target below a known parent",
            k0(),
            zone(&["k9", "k0"]),
        ),
        ("unknown source", zone(&["k9", "k0"]), k0()),
        ("target in a disjoint tree", k0(), zone(&["z1", "z0"])),
    ];

    for (name, source, target) in vectors {
        let decision = engine.decide_route(&routable(source, target));
        assert_eq!(
            decision.denial_reason(),
            Some(ZoneRouteFailClosedReason::UnknownParent),
            "unknown-zone vector: {name}"
        );
        assert_eq!(
            decision.audit_event(),
            ZoneRouteAuditEventKind::ZoneRouteDenied,
            "unknown-zone audit event: {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Class: loop and multi-parent detection
// ---------------------------------------------------------------------------

/// A repeated label deeper in the tree is a distinct Zone path, not a cycle,
/// so it must be admitted rather than mistaken for a loop.
///
/// The engine's `Loop` reason, like `SiblingOrParentRouteAdvert`, guards the
/// deserialization path: `ZoneTreeEdge` already refuses any edge whose child
/// is not the direct child of its parent, so a validated advertisement cannot
/// close a parent cycle. This vector pins the boundary from the admissible
/// side, which is the side a false positive would break.
#[test]
fn loop_detection_vectors() {
    let mut engine = k0_k1_k2_k3_engine();

    let advert = AdvertVector::new(k2(), k3())
        .route(
            "route-repeat",
            zone(&["k1", "k3", "k2", "k1", "k0"]),
            "k1",
            &["get"],
        )
        .window(1_200, 4_000)
        .signature_ref("sigref-repeat")
        .generation("gen-3")
        .build();
    let alloc = allocation(k2(), k3(), "gen-3", vec![k3()], 8, &["get"]);
    let outcome = engine.admit_advertisement(&advert, &alloc, NOW);
    assert!(
        matches!(outcome, ZoneAdvertisementAdmission::Accepted { .. }),
        "a repeated label at a deeper position is a distinct Zone: {outcome:?}"
    );

    // The reused label must not have collapsed the two Zones together.
    let deep = zone(&["k1", "k3", "k2", "k1", "k0"]);
    let ZoneRouteDecision::Allowed { path, .. } =
        engine.decide_route(&routable(k0(), deep.clone()))
    else {
        panic!("expected the deep repeated-label Zone to be routable");
    };
    assert_eq!(path.hop_count(), 4);
    assert_eq!(path.target_zone(), &deep);
    assert_ne!(path.target_zone(), &k1());
}

/// Two different advertisers claiming the same descendant is refused as
/// multi-parent, and the first claim survives intact.
#[test]
fn multi_parent_detection_vectors() {
    let mut engine = k0_k1_k2_engine();

    // A second child of K0 advertises its own subtree. Nothing collides, so
    // this must be admitted: the conflict class is about one descendant with
    // two claimants, not about two claimants existing.
    let sibling = zone(&["k5", "k0"]);
    let advert = AdvertVector::new(k0(), sibling.clone())
        .route("route-steal", zone(&["k2", "k5", "k0"]), "k2", &["get"])
        .window(1_200, 4_000)
        .signature_ref("sigref-sibling")
        .generation("gen-5")
        .build();
    let alloc = allocation(k0(), sibling.clone(), "gen-5", vec![sibling], 8, &["get"]);
    assert!(
        matches!(
            engine.admit_advertisement(&advert, &alloc, NOW),
            ZoneAdvertisementAdmission::Accepted { .. }
        ),
        "a distinct Zone path under a different parent is not a conflict"
    );

    // The genuine conflict: K1 claims K3 with next hop K2, and then K2 claims
    // the same K3 with next hop K3. One descendant, two advertisers.
    let mut engine = k0_k1_k2_engine();
    let far = AdvertVector::new(k0(), k1())
        .route("route-k2", k2(), "k2", &["get"])
        .route("route-k3-far", k3(), "k2", &["get"])
        .window(1_200, 4_000)
        .signature_ref("sigref-far")
        .build();
    let far_alloc = allocation(k0(), k1(), "gen-1", vec![k1()], 8, &["get"]);
    assert_accepted(
        engine.admit_advertisement(&far, &far_alloc, NOW),
        &["route-k2", "route-k3-far"],
    );

    let near = AdvertVector::new(k1(), k2())
        .route("route-k3-near", k3(), "k3", &["get"])
        .window(1_300, 4_000)
        .signature_ref("sigref-near")
        .generation("gen-2")
        .build();
    let near_alloc = allocation(k1(), k2(), "gen-2", vec![k2()], 8, &["get"]);
    assert_eq!(
        engine
            .admit_advertisement(&near, &near_alloc, NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::MultiParent),
        "a second advertiser for a live descendant is multi-parent"
    );

    // The refused advertisement must not have partially landed.
    let inventory = engine.route_inventory();
    assert_eq!(inventory.len(), 2, "the original claims must survive");
    let k3_row = inventory
        .iter()
        .find(|entry| entry.descendant == k3())
        .expect("the K3 row is still projected");
    assert_eq!(k3_row.advertising_zone, k1());
    assert_eq!(k3_row.route_id, route_id("route-k3-far"));
}

// ---------------------------------------------------------------------------
// Class: capability ceiling
// ---------------------------------------------------------------------------

/// One capability-ceiling vector.
struct CapabilityVector {
    name: &'static str,
    target: ZonePath,
    required: Option<&'static str>,
    expected: Option<ZoneRouteFailClosedReason>,
}

#[test]
fn capability_ceiling_vectors() {
    // K1 advertises K2 with {get, list}; K2 advertises K3 with {get}. The
    // ceiling therefore narrows monotonically as the walk descends.
    let engine = k0_k1_k2_k3_engine();
    let vectors = vec![
        CapabilityVector {
            name: "no capability requirement is always satisfied",
            target: k3(),
            required: None,
            expected: None,
        },
        CapabilityVector {
            name: "a capability every hop advertises survives the walk",
            target: k3(),
            required: Some("get"),
            expected: None,
        },
        CapabilityVector {
            name: "a capability dropped by a deeper hop is refused",
            target: k3(),
            required: Some("list"),
            expected: Some(ZoneRouteFailClosedReason::MissingCapability),
        },
        CapabilityVector {
            name: "a capability the nearer hop still carries is allowed there",
            target: k2(),
            required: Some("list"),
            expected: None,
        },
        CapabilityVector {
            name: "a capability nobody advertised is refused",
            target: k2(),
            required: Some("delete"),
            expected: Some(ZoneRouteFailClosedReason::MissingCapability),
        },
        CapabilityVector {
            name: "the local root asserts no advertised ceiling",
            target: k0(),
            required: Some("delete"),
            expected: None,
        },
    ];

    for vector in vectors {
        let mut request = routable(k0(), vector.target.clone());
        request.required_capability = vector.required.map(capability);
        let decision = engine.decide_route(&request);
        assert_eq!(
            decision.denial_reason(),
            vector.expected,
            "capability vector: {}",
            vector.name
        );
    }
}

/// The narrowed ceiling is reported on the allowed decision, not merely used
/// internally, so a caller can enforce it independently.
#[test]
fn effective_capability_reporting_vectors() {
    let engine = k0_k1_k2_k3_engine();

    let ZoneRouteDecision::Allowed {
        effective_capabilities,
        ..
    } = engine.decide_route(&routable(k0(), k3()))
    else {
        panic!("expected an allowed route to K3");
    };
    assert_eq!(
        effective_capabilities,
        Some(caps(&["get"])),
        "the deepest hop's narrower set is the effective ceiling"
    );

    let ZoneRouteDecision::Allowed {
        effective_capabilities,
        ..
    } = engine.decide_route(&routable(k0(), k2()))
    else {
        panic!("expected an allowed route to K2");
    };
    assert_eq!(
        effective_capabilities,
        Some(caps(&["get", "list"])),
        "a shallower target keeps the wider advertised set"
    );

    let ZoneRouteDecision::Allowed {
        effective_capabilities,
        ..
    } = engine.decide_route(&routable(k0(), k0()))
    else {
        panic!("expected an allowed local-root route");
    };
    assert_eq!(
        effective_capabilities, None,
        "the local root reports no advertised ceiling at all"
    );
}

// ---------------------------------------------------------------------------
// Class: replay window
// ---------------------------------------------------------------------------

#[test]
fn replay_window_vectors() {
    // A byte-identical re-presentation inside the live window is a replay.
    let mut engine = ZoneRouteEngine::new(k0());
    let (advert, alloc) = k1_advertisement();
    assert_accepted(
        engine.admit_advertisement(&advert, &alloc, NOW),
        &["route-k2"],
    );
    let (replayed, replayed_alloc) = k1_advertisement();
    assert_eq!(
        engine
            .admit_advertisement(&replayed, &replayed_alloc, NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::Replay),
        "an identical advertisement inside its own window is a replay"
    );

    // A renewal that advances both the issue time and the signature reference
    // is admitted.
    let renewal = AdvertVector::new(k0(), k1())
        .route("route-k2", k2(), "k2", &["get", "list"])
        .window(1_400, 5_000)
        .signature_ref("sigref-renewed")
        .build();
    assert_accepted(
        engine.admit_advertisement(&renewal, &alloc, NOW),
        &["route-k2"],
    );

    // A stale re-presentation that does not advance its issue time is refused
    // even with a fresh signature reference.
    let stale = AdvertVector::new(k0(), k1())
        .route("route-k2", k2(), "k2", &["get", "list"])
        .window(1_400, 5_000)
        .signature_ref("sigref-stale")
        .build();
    assert_eq!(
        engine
            .admit_advertisement(&stale, &alloc, NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::Replay),
        "an advertisement that does not advance its issue time is a replay"
    );
}

/// Expiry sweeps the projection and makes a previously known Zone unknown
/// again, which is the observable end of the replay window.
#[test]
fn expiry_sweep_vectors() {
    let mut engine = k0_k1_k2_engine();
    assert!(
        engine
            .decide_route(&routable(k0(), k2()))
            .denial_reason()
            .is_none()
    );

    let report = engine.prune_expired(NOW);
    assert_eq!(
        report.route_entries, 0,
        "nothing is expired while the window is live"
    );

    let report = engine.prune_expired(4_000);
    assert!(
        report.route_entries >= 1,
        "the expired route row is reclaimed"
    );
    assert!(
        report.replay_keys >= 1,
        "the expired replay key is reclaimed"
    );
    assert!(
        engine.route_inventory().is_empty(),
        "the projection is empty after expiry"
    );

    let mut request = routable(k0(), k2());
    request.current_time_unix_seconds = 4_000;
    assert_eq!(
        engine.decide_route(&request).denial_reason(),
        Some(ZoneRouteFailClosedReason::UnknownParent),
        "an expired Zone is unknown again"
    );
}

// ---------------------------------------------------------------------------
// Class: withdrawal
// ---------------------------------------------------------------------------

fn withdrawal(
    advertising_zone: ZonePath,
    generation_value: &str,
    ids: &[&str],
    issued_at: u64,
) -> ZoneLinkRouteWithdrawal {
    ZoneLinkRouteWithdrawal::new(
        ZONE_ROUTING_SCHEMA_VERSION,
        advertising_zone,
        generation(generation_value),
        ids.iter().map(|id| route_id(id)).collect(),
        issued_at,
        signature("sigref-withdraw"),
    )
    .expect("valid withdrawal")
}

#[test]
fn withdrawal_vectors() {
    let mut engine = k0_k1_k2_k3_engine();

    // Withdrawing an unknown identifier is idempotent, not an error.
    let outcome =
        engine.admit_withdrawal(&withdrawal(k1(), "gen-1", &["route-absent"], 1_200), NOW);
    assert_eq!(
        outcome,
        ZoneWithdrawalAdmission::Accepted {
            withdrawn_route_ids: Vec::new()
        },
        "withdrawing an unknown route removes nothing and refuses nothing"
    );

    // A withdrawal from the wrong generation is refused and changes nothing.
    assert_eq!(
        engine
            .admit_withdrawal(&withdrawal(k1(), "gen-9", &["route-k2"], 1_200), NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::NamespaceViolation)
    );
    assert_eq!(engine.route_inventory().len(), 2);

    // A withdrawal from a different Zone is refused and changes nothing.
    assert_eq!(
        engine
            .admit_withdrawal(&withdrawal(k2(), "gen-1", &["route-k2"], 1_200), NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert)
    );
    assert_eq!(engine.route_inventory().len(), 2);

    // A future-dated withdrawal is malformed.
    assert_eq!(
        engine
            .admit_withdrawal(&withdrawal(k1(), "gen-1", &["route-k2"], 9_000), NOW)
            .denial_reason(),
        Some(ZoneRouteFailClosedReason::MalformedAdvert)
    );

    // The matching withdrawal removes exactly the named row.
    let outcome = engine.admit_withdrawal(&withdrawal(k1(), "gen-1", &["route-k2"], 1_200), NOW);
    assert_eq!(
        outcome,
        ZoneWithdrawalAdmission::Accepted {
            withdrawn_route_ids: vec![route_id("route-k2")]
        }
    );
    assert_eq!(
        outcome.audit_event(),
        ZoneRouteAuditEventKind::ZoneAdvertisementWithdrawn
    );
    let remaining = engine.route_inventory();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].route_id, route_id("route-k3"));
}

// ---------------------------------------------------------------------------
// Class: K0/K1/K2 topology scenario
// ---------------------------------------------------------------------------

/// The end-to-end K0/K1/K2 call the specification walks through: K0 decides a
/// two-hop route to K2, K1 relays it with the budget decremented, and K2
/// dispatches locally.
#[test]
fn k0_k1_k2_topology_scenario() {
    let engine = k0_k1_k2_engine();

    let mut request = routable(k0(), k2());
    request.required_capability = Some(capability("get"));
    let ZoneRouteDecision::Allowed {
        path,
        effective_capabilities,
        remaining_hops_after,
    } = engine.decide_route(&request)
    else {
        panic!("expected the K0 to K2 route to be allowed");
    };
    assert_eq!(path.nearest_common_ancestor(), &k0());
    assert_eq!(path.hop_count(), 2);
    assert_eq!(path.source_zone(), &k0());
    assert_eq!(path.target_zone(), &k2());
    assert_eq!(path.hops()[0].to(), &k1(), "the first hop lands in K1");
    assert_eq!(path.hops()[1].to(), &k2(), "the second hop lands in K2");
    assert_eq!(effective_capabilities, Some(caps(&["get", "list"])));
    assert_eq!(
        remaining_hops_after,
        ZONE_ROUTE_INITIAL_HOP_BUDGET - 2,
        "the caller pays both hops up front"
    );

    // K1 forwards the frame. Both grants are independent and both are needed.
    let mut relay = ZoneRelayRequest::new(remaining_hops_after);
    relay.relay_granted = true;
    relay.target_verb_granted = true;
    relay.zone_link_connected = true;
    let admission = ZoneRouteEngine::admit_relay_hop(&relay);
    assert_eq!(
        admission,
        ZoneRelayAdmission::Admitted {
            forwarded_remaining_hops: remaining_hops_after - 1
        }
    );
    assert_eq!(
        admission.audit_event(),
        ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted
    );

    // The projection K0 exposes names exactly the K1-advertised route to K2.
    let inventory = engine.route_inventory();
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].descendant, k2());
    assert_eq!(inventory[0].advertising_zone, k1());
    assert_eq!(
        inventory[0].next_hop_child,
        ZoneLabelId::parse("k2").expect("valid label")
    );
}

/// One relay-grant vector: relay authority alone never carries the target verb,
/// and the target verb alone never carries relay authority.
struct RelayVector {
    name: &'static str,
    request: ZoneRelayRequest,
    expected: Option<ZoneRouteFailClosedReason>,
}

#[test]
fn relay_grant_vectors() {
    let full = || {
        let mut request = ZoneRelayRequest::new(4);
        request.relay_granted = true;
        request.target_verb_granted = true;
        request.zone_link_connected = true;
        request
    };

    let vectors = vec![
        RelayVector {
            name: "both independent grants present",
            request: full(),
            expected: None,
        },
        RelayVector {
            name: "refusing defaults deny before anything else",
            request: ZoneRelayRequest::new(4),
            expected: Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected),
        },
        RelayVector {
            name: "relay grant missing",
            request: {
                let mut request = full();
                request.relay_granted = false;
                request
            },
            expected: Some(ZoneRouteFailClosedReason::RelayDenied),
        },
        RelayVector {
            name: "target verb grant missing",
            request: {
                let mut request = full();
                request.target_verb_granted = false;
                request
            },
            expected: Some(ZoneRouteFailClosedReason::PolicyDenial),
        },
        RelayVector {
            name: "uplink down",
            request: {
                let mut request = full();
                request.zone_link_connected = false;
                request
            },
            expected: Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected),
        },
        RelayVector {
            name: "a descriptor attachment is never relayable",
            request: {
                let mut request = full();
                request.offers_attachment = true;
                request
            },
            expected: Some(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink),
        },
        RelayVector {
            name: "an exhausted budget cannot be forwarded",
            request: {
                let mut request = full();
                request.arrived_remaining_hops = 0;
                request
            },
            expected: Some(ZoneRouteFailClosedReason::HopLimitExceeded),
        },
    ];

    for vector in vectors {
        let admission = ZoneRouteEngine::admit_relay_hop(&vector.request);
        assert_eq!(
            admission.denial_reason(),
            vector.expected,
            "relay vector: {}",
            vector.name
        );
        let expected_event = if vector.expected.is_none() {
            ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted
        } else {
            ZoneRouteAuditEventKind::ZoneLinkRelayDenied
        };
        assert_eq!(
            admission.audit_event(),
            expected_event,
            "relay audit event: {}",
            vector.name
        );
    }
}

// ---------------------------------------------------------------------------
// Class: hop-count boundary
// ---------------------------------------------------------------------------

/// One hop-count boundary vector: a target whose path costs `path_hops`, a
/// budget, and whether that budget is exactly enough.
struct HopBoundaryVector {
    name: &'static str,
    target: ZonePath,
    path_hops: u32,
    remaining_hops: u32,
    expected: Option<ZoneRouteFailClosedReason>,
}

#[test]
fn hop_count_boundary_vectors() {
    let engine = k0_k1_k2_k3_engine();
    let vectors = vec![
        HopBoundaryVector {
            name: "zero budget refuses before the walk even starts",
            target: k1(),
            path_hops: 1,
            remaining_hops: 0,
            expected: Some(ZoneRouteFailClosedReason::HopLimitExceeded),
        },
        HopBoundaryVector {
            name: "one hop with exactly one hop of budget",
            target: k1(),
            path_hops: 1,
            remaining_hops: 1,
            expected: None,
        },
        HopBoundaryVector {
            name: "two hops with one hop of budget is one short",
            target: k2(),
            path_hops: 2,
            remaining_hops: 1,
            expected: Some(ZoneRouteFailClosedReason::HopLimitExceeded),
        },
        HopBoundaryVector {
            name: "two hops with exactly two hops of budget",
            target: k2(),
            path_hops: 2,
            remaining_hops: 2,
            expected: None,
        },
        HopBoundaryVector {
            name: "three hops with two hops of budget is one short",
            target: k3(),
            path_hops: 3,
            remaining_hops: 2,
            expected: Some(ZoneRouteFailClosedReason::HopLimitExceeded),
        },
        HopBoundaryVector {
            name: "three hops with exactly three hops of budget",
            target: k3(),
            path_hops: 3,
            remaining_hops: 3,
            expected: None,
        },
        HopBoundaryVector {
            name: "the protocol initial budget covers the deepest seeded path",
            target: k3(),
            path_hops: 3,
            remaining_hops: ZONE_ROUTE_INITIAL_HOP_BUDGET,
            expected: None,
        },
    ];

    for vector in vectors {
        let mut request = routable(k0(), vector.target.clone());
        request.remaining_hops = vector.remaining_hops;
        let decision = engine.decide_route(&request);
        assert_eq!(
            decision.denial_reason(),
            vector.expected,
            "hop boundary vector: {}",
            vector.name
        );
        if let ZoneRouteDecision::Allowed {
            path,
            remaining_hops_after,
            ..
        } = decision
        {
            assert_eq!(
                path.hop_count() as u32,
                vector.path_hops,
                "hop cost: {}",
                vector.name
            );
            assert_eq!(
                remaining_hops_after,
                vector.remaining_hops - vector.path_hops,
                "budget after: {}",
                vector.name
            );
        }
    }
}

/// A relayed frame's budget strictly decreases and can never exceed the
/// protocol initial budget, so a chain of relays terminates.
#[test]
fn relay_budget_monotonicity_vector() {
    let mut remaining = ZONE_ROUTE_INITIAL_HOP_BUDGET;
    let mut hops = 0_u32;
    loop {
        let mut request = ZoneRelayRequest::new(remaining);
        request.relay_granted = true;
        request.target_verb_granted = true;
        request.zone_link_connected = true;
        match ZoneRouteEngine::admit_relay_hop(&request) {
            ZoneRelayAdmission::Admitted {
                forwarded_remaining_hops,
            } => {
                assert!(
                    forwarded_remaining_hops < remaining,
                    "a forwarded budget must strictly decrease"
                );
                remaining = forwarded_remaining_hops;
                hops += 1;
                assert!(
                    hops <= ZONE_ROUTE_INITIAL_HOP_BUDGET,
                    "the relay chain must terminate within the initial budget"
                );
            }
            ZoneRelayAdmission::Denied { reason } => {
                assert_eq!(reason, ZoneRouteFailClosedReason::HopLimitExceeded);
                break;
            }
        }
    }
    assert_eq!(hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);
    assert!(
        (hops as usize) <= MAX_ZONE_ROUTE_PATH_HOPS,
        "the initial budget stays within the path bound"
    );
}

// ---------------------------------------------------------------------------
// Class: pre-decision refusal ordering
// ---------------------------------------------------------------------------

/// The refusal order is itself a contract: connectivity, then authorization,
/// then the budget, then the tree. Each row removes exactly one input.
#[test]
fn refusal_ordering_vectors() {
    let engine = k0_k1_k2_engine();
    let vectors: Vec<(&str, ZoneRouteRequest, ZoneRouteFailClosedReason)> = vec![
        (
            "an all-defaults request refuses on connectivity first",
            ZoneRouteRequest::new(k0(), k2(), NOW),
            ZoneRouteFailClosedReason::ZoneLinkDisconnected,
        ),
        (
            "a connected but unauthorized request refuses on policy",
            {
                let mut request = ZoneRouteRequest::new(k0(), k2(), NOW);
                request.zone_link_connected = true;
                request
            },
            ZoneRouteFailClosedReason::PolicyDenial,
        ),
        (
            "an authorized request with no budget refuses on the hop limit",
            {
                let mut request = routable(k0(), k2());
                request.remaining_hops = 0;
                request
            },
            ZoneRouteFailClosedReason::HopLimitExceeded,
        ),
        (
            "a fully authorized request with budget reaches the tree walk",
            routable(k0(), zone(&["k9", "k0"])),
            ZoneRouteFailClosedReason::UnknownParent,
        ),
    ];

    for (name, request, expected) in vectors {
        assert_eq!(
            engine.decide_route(&request).denial_reason(),
            Some(expected),
            "refusal ordering vector: {name}"
        );
    }
}

/// The default request refuses on every safety-relevant input at once, so a
/// caller that forgets to populate one gets a refusal rather than a permissive
/// answer.
#[test]
fn refusing_default_vectors() {
    let request = ZoneRouteRequest::new(k0(), k2(), NOW);
    assert!(!request.policy_allows);
    assert!(!request.zone_link_connected);
    assert!(request.required_capability.is_none());
    assert_eq!(request.remaining_hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);

    let relay = ZoneRelayRequest::new(ZONE_ROUTE_INITIAL_HOP_BUDGET);
    assert!(!relay.relay_granted);
    assert!(!relay.target_verb_granted);
    assert!(!relay.zone_link_connected);
    assert!(!relay.offers_attachment);
}
