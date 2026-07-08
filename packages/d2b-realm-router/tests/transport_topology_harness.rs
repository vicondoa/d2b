use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2b_realm_core::{
    Capability, CapabilitySet, ControllerGenerationId, CorrelationId, DiscoveryIngressClass,
    DiscoveryQueueDropPolicy, DiscoveryQueuePolicy, KeyFingerprint, OperationId, OperationKind,
    PreAuthAdmissionOutcome, ProtocolToken, RealmId, RealmPath, RealmTreeEdge,
    ReplayWindowMetadata, RouteAdvertisement, RouteAdvertisementEnvelope, RouteFailClosedReason,
    RouteId, RoutePolicyRuleId, RouteReplayWindowId, RouteSignature, SignatureRef,
    TreeRouteDecision, TreeRouteDecisionOutcome, TreeRouteHop, TreeRouteHopDirection,
    TreeRoutePath, UnverifiedPeerAdmissionAttemptMetadata, UnverifiedPeerRef,
};
use d2b_realm_provider::provider::TransportProvider;
use d2b_realm_provider::types::{NodeRegistration, TransportTarget};
use d2b_realm_transport::LoopbackTransport;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
struct MockRealm {
    parent: Option<RealmPath>,
    capabilities: CapabilitySet,
}

#[derive(Debug, Clone)]
struct MockDiscoveryQueue {
    policy: DiscoveryQueuePolicy,
    queued: VecDeque<UnverifiedPeerRef>,
    seen_by_peer: BTreeMap<String, u32>,
    next_attempt: u64,
}

impl MockDiscoveryQueue {
    fn new(policy: DiscoveryQueuePolicy) -> Self {
        Self {
            policy,
            queued: VecDeque::new(),
            seen_by_peer: BTreeMap::new(),
            next_attempt: 1,
        }
    }

    fn admit(
        &mut self,
        peer: UnverifiedPeerRef,
        ingress_class: DiscoveryIngressClass,
        claimed_realm: Option<RealmPath>,
    ) -> UnverifiedPeerAdmissionAttemptMetadata {
        let count = self
            .seen_by_peer
            .entry(peer.as_str().to_owned())
            .or_insert(0);
        let outcome = if *count >= self.policy.per_unverified_peer_rate_limit_per_minute {
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::RateLimited,
            }
        } else if self.queued.len() >= self.policy.max_depth as usize {
            PreAuthAdmissionOutcome::Dropped {
                reason: RouteFailClosedReason::QueueFullDropNew,
            }
        } else {
            *count += 1;
            self.queued.push_back(peer.clone());
            PreAuthAdmissionOutcome::Queued
        };

        let attempt_id = OperationId::parse(format!("attempt-{}", self.next_attempt)).unwrap();
        let correlation_id = CorrelationId::parse(format!("corr-{}", self.next_attempt)).unwrap();
        self.next_attempt += 1;
        UnverifiedPeerAdmissionAttemptMetadata::new(
            attempt_id,
            correlation_id,
            None,
            peer,
            ingress_class,
            claimed_realm,
            self.queued.len() as u32,
            outcome,
        )
        .unwrap()
    }

    fn queued_len(&self) -> usize {
        self.queued.len()
    }
}

#[derive(Debug, Clone)]
struct MockReplayWindow {
    max_entries: u32,
    ttl_seconds: u64,
    opened_at_unix_seconds: u64,
    now_unix_seconds: u64,
    observed_replays: u32,
    seen: BTreeMap<String, u64>,
}

impl MockReplayWindow {
    fn new(max_entries: u32, ttl_seconds: u64, opened_at_unix_seconds: u64) -> Self {
        Self {
            max_entries,
            ttl_seconds,
            opened_at_unix_seconds,
            now_unix_seconds: opened_at_unix_seconds,
            observed_replays: 0,
            seen: BTreeMap::new(),
        }
    }

    fn metadata(&self) -> ReplayWindowMetadata {
        ReplayWindowMetadata::new(
            RouteReplayWindowId::parse("replay-window-1").unwrap(),
            self.max_entries,
            self.seen.len() as u32,
            self.ttl_seconds,
            self.observed_replays,
            self.opened_at_unix_seconds,
        )
        .unwrap()
    }

    fn admit(
        &mut self,
        key: String,
    ) -> Result<ReplayWindowMetadata, (RouteFailClosedReason, ReplayWindowMetadata)> {
        self.seen
            .retain(|_, first_seen| self.now_unix_seconds - *first_seen <= self.ttl_seconds);
        if self.seen.contains_key(&key) {
            self.observed_replays += 1;
            return Err((RouteFailClosedReason::Replay, self.metadata()));
        }
        if self.seen.len() >= self.max_entries as usize {
            return Err((RouteFailClosedReason::RateLimited, self.metadata()));
        }
        self.seen.insert(key, self.now_unix_seconds);
        Ok(self.metadata())
    }
}

#[derive(Debug)]
struct MockTopology {
    realms: BTreeMap<RealmPath, MockRealm>,
    direct_shortcut_denials: BTreeSet<(RealmPath, RealmPath)>,
    policy_rule: RoutePolicyRuleId,
    discovery: MockDiscoveryQueue,
    replay: MockReplayWindow,
    next_decision: u64,
}

impl MockTopology {
    fn new() -> Self {
        let mut topology = Self {
            realms: BTreeMap::new(),
            direct_shortcut_denials: BTreeSet::new(),
            policy_rule: RoutePolicyRuleId::parse("policy-deny-shortcut").unwrap(),
            discovery: MockDiscoveryQueue::new(
                DiscoveryQueuePolicy::new(8, 8, 60, 60, DiscoveryQueueDropPolicy::DropNew).unwrap(),
            ),
            replay: MockReplayWindow::new(8, 60, 1),
            next_decision: 1,
        };
        topology.add_root(realm(&["local"]), CapabilitySet::empty());
        topology
    }

    fn with_discovery_policy(mut self, policy: DiscoveryQueuePolicy) -> Self {
        self.discovery = MockDiscoveryQueue::new(policy);
        self
    }

    fn add_root(&mut self, path: RealmPath, capabilities: CapabilitySet) {
        self.realms.insert(
            path,
            MockRealm {
                parent: None,
                capabilities,
            },
        );
    }

    fn add_child(&mut self, parent: RealmPath, child: RealmPath, capabilities: CapabilitySet) {
        RealmTreeEdge::new(parent.clone(), child.clone()).expect("direct child edge");
        self.realms.insert(
            child,
            MockRealm {
                parent: Some(parent),
                capabilities,
            },
        );
    }

    fn deny_direct_shortcut(&mut self, source: RealmPath, target: RealmPath) {
        self.direct_shortcut_denials.insert((source, target));
    }

    fn admit_discovery(
        &mut self,
        peer: &str,
        claimed_realm: Option<RealmPath>,
    ) -> UnverifiedPeerAdmissionAttemptMetadata {
        self.discovery.admit(
            UnverifiedPeerRef::parse(peer).unwrap(),
            DiscoveryIngressClass::ParentRelay,
            claimed_realm,
        )
    }

    fn accept_advertisement(
        &mut self,
        peer: &str,
        advertisement: RouteAdvertisement,
    ) -> Result<RouteAdvertisementEnvelope, (RouteFailClosedReason, ReplayWindowMetadata)> {
        if self.replay.now_unix_seconds >= advertisement.expires_at_unix_seconds {
            return Err((RouteFailClosedReason::Expired, self.replay.metadata()));
        }
        let replay_key = advertisement_key(&advertisement);
        let replay_window = self.replay.admit(replay_key)?;
        let admission = self.admit_discovery(peer, Some(advertisement.advertising_realm.clone()));
        if !matches!(admission.outcome, PreAuthAdmissionOutcome::Queued) {
            return Err((
                match admission.outcome {
                    PreAuthAdmissionOutcome::Queued => unreachable!(),
                    PreAuthAdmissionOutcome::Dropped { reason } => reason,
                },
                replay_window,
            ));
        }
        for route in &advertisement.routes {
            self.realms
                .entry(route.descendant.clone())
                .and_modify(|realm| realm.capabilities = route.capabilities.clone())
                .or_insert_with(|| MockRealm {
                    parent: Some(advertisement.advertising_realm.clone()),
                    capabilities: route.capabilities.clone(),
                });
        }
        Ok(RouteAdvertisementEnvelope {
            admission,
            replay_window,
            correlation_id: CorrelationId::parse("advert-corr-1").unwrap(),
            trace: None,
            advertisement,
            received_at_unix_seconds: self.replay.now_unix_seconds,
        })
    }

    fn decide_route(
        &mut self,
        source: RealmPath,
        target: RealmPath,
        operation_kind: OperationKind,
        direct_shortcut: bool,
    ) -> TreeRouteDecision {
        let decision_id = OperationId::parse(format!("decision-{}", self.next_decision)).unwrap();
        let correlation_id = CorrelationId::parse(format!("route-corr-{}", self.next_decision))
            .expect("valid route correlation id");
        self.next_decision += 1;
        let required_capability = operation_kind.required_capability();

        let denied = if direct_shortcut
            && self
                .direct_shortcut_denials
                .contains(&(source.clone(), target.clone()))
        {
            Some(RouteFailClosedReason::PolicyDenial)
        } else if required_capability.is_some_and(|capability| {
            !self
                .realms
                .get(&target)
                .is_some_and(|realm| realm.capabilities.has(capability))
        }) {
            Some(RouteFailClosedReason::MissingCapability)
        } else {
            None
        };

        let (policy_rule_id, outcome) = if let Some(reason) = denied {
            (
                (reason == RouteFailClosedReason::PolicyDenial).then(|| self.policy_rule.clone()),
                TreeRouteDecisionOutcome::Denied { reason },
            )
        } else {
            (
                None,
                TreeRouteDecisionOutcome::Allowed {
                    path: self.tree_path(source.clone(), target.clone()),
                },
            )
        };

        TreeRouteDecision {
            decision_id,
            correlation_id,
            trace: None,
            source_realm: source,
            target_realm: target,
            operation_kind,
            required_capability,
            policy_rule_id,
            outcome,
        }
    }

    fn tree_path(&self, source: RealmPath, target: RealmPath) -> TreeRoutePath {
        let ancestor = nearest_common_ancestor(&source, &target);
        let mut hops = Vec::new();

        let mut current = source.clone();
        while current != ancestor {
            let parent = self.parent_of(&current);
            hops.push(
                TreeRouteHop::new(
                    current.clone(),
                    parent.clone(),
                    RealmTreeEdge::new(parent.clone(), current.clone()).unwrap(),
                    TreeRouteHopDirection::UpToParent,
                    Some(route_id("route-up", &current)),
                )
                .unwrap(),
            );
            current = parent;
        }

        let mut down = Vec::new();
        let mut current = target.clone();
        while current != ancestor {
            let parent = self.parent_of(&current);
            down.push((parent.clone(), current.clone()));
            current = parent;
        }
        for (parent, child) in down.into_iter().rev() {
            hops.push(
                TreeRouteHop::new(
                    parent.clone(),
                    child.clone(),
                    RealmTreeEdge::new(parent, child.clone()).unwrap(),
                    TreeRouteHopDirection::DownToChild,
                    Some(route_id("route-down", &child)),
                )
                .unwrap(),
            );
        }

        TreeRoutePath::new(source, target, ancestor, hops).unwrap()
    }

    fn parent_of(&self, realm: &RealmPath) -> RealmPath {
        self.realms
            .get(realm)
            .and_then(|realm| realm.parent.clone())
            .expect("realm has parent in mock topology")
    }
}

fn realm(labels: &[&str]) -> RealmPath {
    RealmPath::new(
        labels
            .iter()
            .map(|label| RealmId::parse(*label).unwrap())
            .collect(),
    )
    .unwrap()
}

fn route_id(prefix: &str, realm: &RealmPath) -> RouteId {
    RouteId::parse(format!(
        "{}-{}",
        prefix,
        realm.target_form().replace('.', "-")
    ))
    .unwrap()
}

fn nearest_common_ancestor(source: &RealmPath, target: &RealmPath) -> RealmPath {
    let mut shared_parent_first = Vec::new();
    for (left, right) in source
        .labels()
        .iter()
        .rev()
        .zip(target.labels().iter().rev())
    {
        if left == right {
            shared_parent_first.push(left.clone());
        } else {
            break;
        }
    }
    shared_parent_first.reverse();
    RealmPath::new(shared_parent_first).expect("test realms share a root")
}

fn fingerprint() -> KeyFingerprint {
    KeyFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn signature(signature_ref: &str) -> RouteSignature {
    RouteSignature {
        algorithm: ProtocolToken::parse("ed25519-v1").unwrap(),
        key_role: d2b_realm_core::RealmKeyRole::ControllerGeneration,
        signing_key_fingerprint: fingerprint(),
        signature_ref: SignatureRef::parse(signature_ref).unwrap(),
    }
}

fn advertisement(
    parent: RealmPath,
    advertiser: RealmPath,
    descendant: RealmPath,
    capabilities: CapabilitySet,
    signature_ref: &str,
) -> RouteAdvertisement {
    RouteAdvertisement::new(
        advertiser.clone(),
        RealmTreeEdge::new(parent, advertiser.clone()).unwrap(),
        ControllerGenerationId::parse("gen-1").unwrap(),
        vec![d2b_realm_core::DescendantRoute {
            route_id: route_id("advert", &descendant),
            descendant: descendant.clone(),
            next_hop_child: descendant.labels()[0].clone(),
            capabilities,
        }],
        1,
        120,
        signature(signature_ref),
    )
    .unwrap()
}

fn advertisement_key(advertisement: &RouteAdvertisement) -> String {
    let route_ids = advertisement
        .routes
        .iter()
        .map(|route| route.route_id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{}:{}:{}:{}",
        advertisement.advertising_realm.target_form(),
        advertisement.controller_generation.as_str(),
        advertisement.signature.signature_ref.as_str(),
        route_ids
    )
}

fn sample_topology() -> MockTopology {
    let mut topology = MockTopology::new();
    let local = realm(&["local"]);
    let dev = realm(&["dev", "local"]);
    let work = realm(&["work", "local"]);
    let api = realm(&["api", "dev", "local"]);
    let db = realm(&["db", "work", "local"]);
    topology.add_child(local.clone(), dev.clone(), CapabilitySet::empty());
    topology.add_child(local.clone(), work.clone(), CapabilitySet::empty());
    topology.add_child(dev.clone(), api.clone(), CapabilitySet::empty());
    topology.add_child(work.clone(), db.clone(), CapabilitySet::empty());
    topology
        .accept_advertisement(
            "peer-dev",
            advertisement(
                local.clone(),
                dev,
                api,
                CapabilitySet::empty().with(Capability::Exec),
                "sig-dev",
            ),
        )
        .unwrap();
    topology
        .accept_advertisement(
            "peer-work",
            advertisement(
                local,
                work,
                db,
                CapabilitySet::empty().with(Capability::Exec),
                "sig-work",
            ),
        )
        .unwrap();
    topology
}

#[test]
fn nested_topology_routes_descendant_after_advertisement() {
    let mut topology = sample_topology();
    let decision = topology.decide_route(
        realm(&["dev", "local"]),
        realm(&["api", "dev", "local"]),
        OperationKind::ExecStart,
        false,
    );

    match decision.outcome {
        TreeRouteDecisionOutcome::Allowed { path } => {
            assert_eq!(path.nearest_common_ancestor, realm(&["dev", "local"]));
            assert_eq!(path.hops.len(), 1);
            assert_eq!(path.hops[0].direction, TreeRouteHopDirection::DownToChild);
        }
        other => panic!("expected allowed route, got {other:?}"),
    }
}

#[test]
fn cross_branch_path_routes_through_nearest_common_ancestor() {
    let mut topology = sample_topology();
    let source = realm(&["api", "dev", "local"]);
    let target = realm(&["db", "work", "local"]);
    let decision = topology.decide_route(
        source.clone(),
        target.clone(),
        OperationKind::ExecStart,
        false,
    );

    match decision.outcome {
        TreeRouteDecisionOutcome::Allowed { path } => {
            assert_eq!(path.source_realm, source);
            assert_eq!(path.target_realm, target);
            assert_eq!(path.nearest_common_ancestor, realm(&["local"]));
            assert_eq!(
                path.hops
                    .iter()
                    .map(|hop| hop.direction)
                    .collect::<Vec<_>>(),
                vec![
                    TreeRouteHopDirection::UpToParent,
                    TreeRouteHopDirection::UpToParent,
                    TreeRouteHopDirection::DownToChild,
                    TreeRouteHopDirection::DownToChild,
                ]
            );
        }
        other => panic!("expected allowed cross-branch route, got {other:?}"),
    }
}

#[test]
fn direct_shortcut_denied_by_policy_without_blocking_tree_route() {
    let mut topology = sample_topology();
    let source = realm(&["api", "dev", "local"]);
    let target = realm(&["db", "work", "local"]);
    topology.deny_direct_shortcut(source.clone(), target.clone());

    let shortcut = topology.decide_route(
        source.clone(),
        target.clone(),
        OperationKind::ExecStart,
        true,
    );
    assert!(matches!(
        shortcut.outcome,
        TreeRouteDecisionOutcome::Denied {
            reason: RouteFailClosedReason::PolicyDenial
        }
    ));
    assert_eq!(
        shortcut
            .policy_rule_id
            .as_ref()
            .map(RoutePolicyRuleId::as_str),
        Some("policy-deny-shortcut")
    );

    let tree = topology.decide_route(source, target, OperationKind::ExecStart, false);
    assert!(matches!(
        tree.outcome,
        TreeRouteDecisionOutcome::Allowed { .. }
    ));
}

#[test]
fn missing_capability_is_a_semantic_route_denial() {
    let mut topology = sample_topology();
    let decision = topology.decide_route(
        realm(&["api", "dev", "local"]),
        realm(&["db", "work", "local"]),
        OperationKind::ExecLogs,
        false,
    );

    assert_eq!(decision.required_capability, Some(Capability::Logs));
    assert!(matches!(
        decision.outcome,
        TreeRouteDecisionOutcome::Denied {
            reason: RouteFailClosedReason::MissingCapability
        }
    ));
}

#[test]
fn discovery_queue_overflow_drops_new_without_evicting_existing_peer() {
    let mut topology = MockTopology::new().with_discovery_policy(
        DiscoveryQueuePolicy::new(1, 8, 60, 60, DiscoveryQueueDropPolicy::DropNew).unwrap(),
    );

    let first = topology.admit_discovery("peer-one", Some(realm(&["local"])));
    let second = topology.admit_discovery("peer-two", Some(realm(&["local"])));

    assert_eq!(first.outcome, PreAuthAdmissionOutcome::Queued);
    assert!(matches!(
        second.outcome,
        PreAuthAdmissionOutcome::Dropped {
            reason: RouteFailClosedReason::QueueFullDropNew
        }
    ));
    assert_eq!(second.queue_depth, 1);
    assert_eq!(topology.discovery.queued_len(), 1);
}

#[test]
fn unauthenticated_peer_rate_limit_drops_new_attempts() {
    let mut topology = MockTopology::new().with_discovery_policy(
        DiscoveryQueuePolicy::new(8, 8, 60, 1, DiscoveryQueueDropPolicy::DropNew).unwrap(),
    );

    let first = topology.admit_discovery("peer-one", Some(realm(&["local"])));
    let second = topology.admit_discovery("peer-one", Some(realm(&["local"])));

    assert_eq!(first.outcome, PreAuthAdmissionOutcome::Queued);
    assert!(matches!(
        second.outcome,
        PreAuthAdmissionOutcome::Dropped {
            reason: RouteFailClosedReason::RateLimited
        }
    ));
    assert_eq!(topology.discovery.queued_len(), 1);
}

#[test]
fn route_advertisement_replay_is_rejected_with_replay_window_metadata() {
    let mut topology = MockTopology::new();
    let local = realm(&["local"]);
    let work = realm(&["work", "local"]);
    let db = realm(&["db", "work", "local"]);
    topology.add_child(local.clone(), work.clone(), CapabilitySet::empty());

    let advert = advertisement(
        local,
        work,
        db,
        CapabilitySet::empty().with(Capability::Exec),
        "sig-work",
    );
    let first = topology
        .accept_advertisement("peer-work", advert.clone())
        .unwrap();
    let replay = topology
        .accept_advertisement("peer-work", advert)
        .unwrap_err();

    assert_eq!(first.replay_window.current_entries, 1);
    assert_eq!(replay.0, RouteFailClosedReason::Replay);
    assert_eq!(replay.1.current_entries, 1);
    assert_eq!(replay.1.observed_replay_count, 1);
}

#[test]
fn expired_route_advertisement_is_rejected_without_queueing_peer() {
    let mut topology = MockTopology::new();
    let local = realm(&["local"]);
    let work = realm(&["work", "local"]);
    let db = realm(&["db", "work", "local"]);
    topology.add_child(local.clone(), work.clone(), CapabilitySet::empty());

    let mut advert = advertisement(
        local,
        work,
        db,
        CapabilitySet::empty().with(Capability::Exec),
        "sig-expired",
    );
    advert.expires_at_unix_seconds = topology.replay.now_unix_seconds;

    let rejected = topology
        .accept_advertisement("peer-work", advert)
        .unwrap_err();
    assert_eq!(rejected.0, RouteFailClosedReason::Expired);
    assert_eq!(topology.discovery.queued_len(), 0);
}

#[test]
fn route_decision_json_has_no_raw_tunnel_semantics() {
    let mut topology = sample_topology();
    let decision = topology.decide_route(
        realm(&["api", "dev", "local"]),
        realm(&["db", "work", "local"]),
        OperationKind::ExecStart,
        false,
    );
    let json = serde_json::to_value(&decision).expect("route decision serializes");

    assert!(json.get("outcome").is_some());
    let encoded = json.to_string();
    assert!(
        encoded.contains("nearestCommonAncestor"),
        "route decision must describe the accountable tree path"
    );
    for forbidden in [
        "tunnel",
        "raw",
        "ssh",
        "vpn",
        "overlay",
        "socket",
        "endpoint",
        "file_descriptor",
        "provider_credential",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "route decision JSON must not expose {forbidden} semantics"
        );
    }
}

#[tokio::test]
async fn topology_harness_uses_in_memory_loopback_only() {
    let transport = LoopbackTransport::new();
    let listener = transport
        .listen(NodeRegistration {
            node: d2b_realm_core::NodeId::parse("gw").unwrap(),
        })
        .await
        .unwrap();
    let (connect, accept) = tokio::join!(
        transport.connect(TransportTarget {
            endpoint: "loopback".to_owned(),
        }),
        listener.accept()
    );
    let mut sender = connect.unwrap();
    let mut receiver = accept.unwrap();

    sender.stream_mut().write_all(b"route-probe").await.unwrap();
    let mut buf = [0_u8; 11];
    receiver.stream_mut().read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"route-probe");
}
