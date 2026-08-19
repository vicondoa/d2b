use d2b_contracts::{
    FeatureFlag, KnownFeatureFlag,
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, LaunchCutoverRunnerRequest,
    },
    public_wire::{HostCutoverOperation, HostCutoverRequest, HostCutoverResetScope},
    types::BundleOpId,
};

#[test]
fn public_cutover_request_round_trips_without_path_fields() {
    let request = HostCutoverRequest {
        operation: HostCutoverOperation::Apply,
        operation_id: Some("op-wire".to_owned()),
        candidate_id: Some("candidate-wire".to_owned()),
        revision_plan_id: Some("plan-wire".to_owned()),
        preview_digest: Some("sha256:".to_owned() + &"a".repeat(64)),
        recovery_digest: Some("sha256:".to_owned() + &"b".repeat(64)),
        operator_id: Some("uid-1000".to_owned()),
        consent_digest: Some("sha256:".to_owned() + &"c".repeat(64)),
        fresh_consent_digest: None,
        reason: None,
        reset_scope: None,
        target: None,
        zone: None,
    };
    let value = serde_json::to_value(&request).expect("serialize request");
    assert!(!value.to_string().contains('/'));
    let decoded: HostCutoverRequest = serde_json::from_value(value).expect("decode request");
    assert_eq!(decoded, request);
}

#[test]
fn reset_authority_is_distinct_from_cutover_authority() {
    let reset = HostCutoverRequest {
        operation: HostCutoverOperation::Reset,
        operation_id: Some("op-reset".to_owned()),
        candidate_id: None,
        revision_plan_id: None,
        preview_digest: None,
        recovery_digest: None,
        operator_id: None,
        consent_digest: None,
        fresh_consent_digest: None,
        reason: None,
        reset_scope: Some(HostCutoverResetScope::Zone),
        target: Some("zone-target".to_owned()),
        zone: Some("zone-target".to_owned()),
    };
    let encoded = serde_json::to_string(&reset).expect("serialize reset");
    assert!(encoded.contains("\"resetScope\":\"zone\""));
    assert!(!encoded.contains("\"operation\":\"apply\""));
}

#[test]
fn launch_runner_broker_request_has_no_spawn_runner_shape() {
    let request = BrokerRequest::LaunchCutoverRunner(LaunchCutoverRunnerRequest {
        operation_id: BundleOpId::new("op-wire"),
        bootstrap_fd_index: 0,
    });
    assert_eq!(request.op_name(), "LaunchCutoverRunner");
    assert!(!matches!(request, BrokerRequest::SpawnRunner(_)));
    let envelope = BrokerRequestEnvelope {
        request,
        caller_role: BrokerCallerRole::AdminUid { uid: 1000 },
        test_peer_uid: None,
        audit_join: None,
    };
    let encoded = serde_json::to_string(&envelope).expect("serialize broker envelope");
    assert!(!encoded.contains("runnerPath"));
    assert!(!encoded.contains("argv"));
}

#[test]
fn cutover_runner_feature_is_explicitly_negotiated() {
    let feature = FeatureFlag::new("cutover-runner-v1").expect("feature");
    assert_eq!(feature.known(), Some(KnownFeatureFlag::CutoverRunnerV1));
}
