use d2b_contract_tests::read_repo_file;
use serde_json::Value;

fn realm_core_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/d2b-realm-core.json",
    ))
    .expect("d2b-realm-core schema parses")
}

fn definition<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema
        .get("definitions")
        .and_then(Value::as_object)
        .and_then(|definitions| definitions.get(name))
        .unwrap_or_else(|| panic!("schema is missing definition {name}"))
}

#[test]
fn route_dtos_are_in_generated_schema_contract() {
    let schema = realm_core_schema();
    let root_refs = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .map(Value::to_string)
        .unwrap_or_else(|| schema.to_string());

    for dto in [
        "SignatureRef",
        "UnverifiedPeerRef",
        "RoutePolicyRuleId",
        "RouteReplayWindowId",
        "ShortcutAuthorizationId",
        "DiscoveryQueueDropPolicy",
        "DiscoveryQueuePolicy",
        "DiscoveryIngressClass",
        "PreAuthAdmissionOutcome",
        "UnverifiedPeerAdmissionAttemptMetadata",
        "ReplayWindowMetadata",
        "SessionAdmissionAttemptMetadata",
        "SessionAdmissionOutcome",
        "RealmTreeEdge",
        "DescendantRoute",
        "RouteSignature",
        "RouteAdvertisement",
        "RouteAdvertisementEnvelope",
        "RouteNamespaceAllocation",
        "TreeRouteHopDirection",
        "TreeRouteHop",
        "TreeRoutePath",
        "RouteFailClosedReason",
        "TreeRouteDecisionOutcome",
        "TreeRouteDecision",
        "DirectShortcutState",
        "DirectShortcutAuthorizationMetadata",
        "DirectShortcutTeardownMetadata",
        "DirectShortcutTeardownReason",
        "RouteAuditEventKind",
        "RouteRealmClass",
        "RoutePlacementClass",
        "RouteAuditLabels",
        "RouteTelemetryCounterKind",
        "RouteTelemetryLabels",
        "RouteTelemetrySample",
        "RouteTelemetryBatch",
    ] {
        definition(&schema, dto);
        assert!(
            root_refs.contains(&format!("#/definitions/{dto}")),
            "d2b-realm-core root schema must expose {dto}"
        );
    }
}

#[test]
fn route_schema_docs_name_tree_routing_roots_and_denials() {
    let schema_docs = read_repo_file("docs/reference/schemas/v2/d2b-realm-core.md");
    let routing_docs = read_repo_file("docs/reference/realm-routing.md");

    for marker in [
        "DiscoveryQueuePolicy",
        "RouteAdvertisementEnvelope",
        "RouteNamespaceAllocation",
        "TreeRouteDecision",
        "DirectShortcutAuthorizationMetadata",
        "RouteTelemetryBatch",
    ] {
        assert!(
            schema_docs.contains(marker),
            "schema reference docs must mention {marker}"
        );
    }

    for marker in [
        "drop-new",
        "replay",
        "rate-limited",
        "missing-capability",
        "policy-denial",
        "raw tunnel",
    ] {
        assert!(
            routing_docs.contains(marker),
            "routing reference docs must mention {marker}"
        );
    }
}
