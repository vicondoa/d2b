//! Route decision benchmark for `ADR046-routing-006`.
//!
//! The work item fixes one number: the p95 latency of a single
//! `ZoneRouteEngine::decide_route` call must stay at or under 1 ms with 1, 10,
//! and 100 active Zone tree entries in the projection. This is a plain
//! `harness = false` benchmark with hand-rolled timing and no benchmark
//! dependency, because the measurement is a per-call latency distribution over
//! a pure in-memory function - there is no I/O to amortize and no statistical
//! machinery needed to see a two-order-of-magnitude budget.
//!
//! What it measures: wall-clock nanoseconds around one `decide_route` call on
//! a pre-built engine, with the target rotated across every advertised
//! descendant so the walk is not answering one hot key. Engine construction,
//! advertisement admission, and target selection all happen outside the timed
//! region. A warmup pass runs before the measured pass so the first-touch page
//! faults and branch mispredictions of a cold projection are not folded into
//! the reported distribution.
//!
//! What it does not measure: throughput, allocation counts, or anything about
//! a real transport. The engine performs no I/O, so this is a CPU-bound
//! measurement of the nearest-common-ancestor walk and the capability
//! intersection over it.

use std::time::{Duration, Instant};

use d2b_contracts::v3::zone_routing::{
    ZONE_ROUTE_INITIAL_HOP_BUDGET, ZONE_ROUTING_SCHEMA_VERSION, ZoneDescendantRoute, ZoneLabelId,
    ZoneLinkControllerGeneration, ZoneLinkNamespaceAllocation, ZoneLinkRouteAdvertisement,
    ZonePath, ZoneRouteCapability, ZoneRouteCapabilitySet, ZoneRouteId, ZoneRouteKeyRole,
    ZoneRouteSignature, ZoneRouteSignatureAlgorithm, ZoneRouteSignatureRef,
    ZoneSigningKeyFingerprint, ZoneTreeEdge,
};
use d2b_zone_routing::engine::{
    ZoneAdvertisementAdmission, ZoneRouteDecision, ZoneRouteEngine, ZoneRouteRequest,
};

/// The p95 budget the work item fixes.
const P95_BUDGET: Duration = Duration::from_millis(1);

/// Active Zone tree entry counts the budget is stated against.
const ENTRY_COUNTS: [usize; 3] = [1, 10, 100];

/// Measured samples per entry count.
const SAMPLES: usize = 20_000;

/// Unmeasured warmup calls per entry count.
const WARMUP: usize = 2_000;

/// The instant every decision is taken at, inside every advertisement window.
const NOW: u64 = 1_500;

fn main() {
    println!("route decision latency (ADR046-routing-006)");
    println!("p95 budget: {P95_BUDGET:?} per decide_route call");

    let mut failures = Vec::new();
    for count in ENTRY_COUNTS {
        let engine = engine_with_entries(count);
        let targets = descendants(count);

        for index in 0..WARMUP {
            let target = &targets[index % targets.len()];
            let decision = engine.decide_route(&request(target.clone()));
            assert_allowed(&decision, target);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for index in 0..SAMPLES {
            let target = targets[index % targets.len()].clone();
            let call = request(target);
            let started = Instant::now();
            let decision = engine.decide_route(&call);
            samples.push(started.elapsed());
            // Consume the result outside the timed region so the call cannot
            // be optimized away, without charging the check to the sample.
            assert!(matches!(decision, ZoneRouteDecision::Allowed { .. }));
        }

        samples.sort_unstable();
        let p50 = percentile(&samples, 50);
        let p95 = percentile(&samples, 95);
        let p99 = percentile(&samples, 99);
        let max = *samples.last().expect("at least one sample");
        println!(
            "  entries={count:>3}  samples={SAMPLES}  p50={p50:?}  p95={p95:?}  p99={p99:?}  max={max:?}"
        );

        if p95 > P95_BUDGET {
            failures.push(format!(
                "entries={count}: p95 {p95:?} exceeds the {P95_BUDGET:?} budget"
            ));
        }
    }

    if failures.is_empty() {
        println!("p95 gate: pass for every measured entry count");
    } else {
        for failure in &failures {
            println!("p95 gate: FAIL - {failure}");
        }
        std::process::exit(1);
    }
}

/// The value at the given percentile of an already sorted sample set.
fn percentile(sorted_samples: &[Duration], percentile: usize) -> Duration {
    assert!(!sorted_samples.is_empty(), "no samples to summarize");
    // Nearest-rank: the smallest sample at or above the requested rank, which
    // never reports a value the run did not actually observe.
    let index = (percentile * sorted_samples.len())
        .div_ceil(100)
        .saturating_sub(1);
    sorted_samples[index.min(sorted_samples.len() - 1)]
}

fn request(target: ZonePath) -> ZoneRouteRequest {
    let mut request = ZoneRouteRequest::new(root(), target, NOW);
    request.policy_allows = true;
    request.zone_link_connected = true;
    request.remaining_hops = ZONE_ROUTE_INITIAL_HOP_BUDGET;
    request.required_capability = Some(capability("get"));
    request
}

fn assert_allowed(decision: &ZoneRouteDecision, target: &ZonePath) {
    assert!(
        matches!(decision, ZoneRouteDecision::Allowed { .. }),
        "benchmark fixture must produce an allowed route, target depth {}",
        target.depth()
    );
}

/// An engine whose projection holds exactly `count` active descendant routes.
///
/// Each entry is a distinct child of the local root advertising one
/// grandchild, so the measured walk is a genuine two-hop
/// nearest-common-ancestor descent rather than a single-edge lookup.
fn engine_with_entries(count: usize) -> ZoneRouteEngine {
    let mut engine = ZoneRouteEngine::new(root());
    for index in 0..count {
        let child = child_zone(index);
        let descendant = descendant_zone(index);
        let advertisement = ZoneLinkRouteAdvertisement::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            child.clone(),
            ZoneTreeEdge::new(root(), child.clone()).expect("direct child edge"),
            generation(index),
            vec![ZoneDescendantRoute::new(
                route_id(index),
                descendant,
                label(&format!("leaf{index}")),
                caps(&["get", "list"]),
            )],
            1_000,
            4_000,
            signature(index),
        )
        .expect("valid advertisement");
        let allocation = ZoneLinkNamespaceAllocation::new(
            ZoneTreeEdge::new(root(), child.clone()).expect("direct child edge"),
            generation(index),
            vec![child],
            8,
            caps(&["get", "list", "watch"]),
        )
        .expect("valid namespace allocation");
        let outcome = engine.admit_advertisement(&advertisement, &allocation, NOW);
        assert!(
            matches!(outcome, ZoneAdvertisementAdmission::Accepted { .. }),
            "benchmark fixture advertisement {index} was refused"
        );
    }
    assert_eq!(
        engine.route_inventory().len(),
        count,
        "benchmark fixture must hold exactly {count} active entries"
    );
    engine
}

fn descendants(count: usize) -> Vec<ZonePath> {
    (0..count).map(descendant_zone).collect()
}

fn root() -> ZonePath {
    ZonePath::new(vec![label("k0")]).expect("valid root path")
}

fn child_zone(index: usize) -> ZonePath {
    ZonePath::new(vec![label(&format!("c{index}")), label("k0")]).expect("valid child path")
}

fn descendant_zone(index: usize) -> ZonePath {
    ZonePath::new(vec![
        label(&format!("leaf{index}")),
        label(&format!("c{index}")),
        label("k0"),
    ])
    .expect("valid descendant path")
}

fn label(value: &str) -> ZoneLabelId {
    ZoneLabelId::parse(value).expect("valid zone label")
}

fn caps(codes: &[&str]) -> ZoneRouteCapabilitySet {
    ZoneRouteCapabilitySet::new(codes.iter().map(|code| capability(code)).collect())
        .expect("valid capability set")
}

fn capability(code: &str) -> ZoneRouteCapability {
    ZoneRouteCapability::parse(code).expect("valid capability")
}

fn route_id(index: usize) -> ZoneRouteId {
    ZoneRouteId::parse(format!("route-{index}")).expect("valid route id")
}

fn generation(index: usize) -> ZoneLinkControllerGeneration {
    ZoneLinkControllerGeneration::parse(format!("gen-{index}")).expect("valid generation")
}

fn signature(index: usize) -> ZoneRouteSignature {
    ZoneRouteSignature::new(
        ZoneRouteSignatureAlgorithm::Ed25519Blake3,
        ZoneRouteKeyRole::ZoneControllerRouting,
        ZoneSigningKeyFingerprint::parse(format!("sha256.{}", "d".repeat(64)))
            .expect("valid fingerprint"),
        ZoneRouteSignatureRef::parse(format!("sigref-{index}")).expect("valid signature ref"),
    )
}
