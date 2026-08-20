use d2b_contracts::{
    FeatureFlag, KnownFeatureFlag,
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, CanonicalAuditDigest,
        CutoverEffectAuthority, CutoverEffectKind, CutoverEffectPayload, CutoverEffectRequest,
        CutoverReplayClass, LaunchCutoverRunnerRequest, ReconcileStorageScopeRequest,
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
        consent_json: None,
        destructive_consent_digest: None,
        destructive_consent_json: None,
        destroy_durable_volumes: None,
        recovery_attestation_json: None,
        host_digest: None,
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
        consent_json: None,
        destructive_consent_digest: None,
        destructive_consent_json: None,
        destroy_durable_volumes: None,
        recovery_attestation_json: None,
        host_digest: None,
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
        capability_digest: CanonicalAuditDigest::parse("sha256:".to_owned() + &"a".repeat(64))
            .expect("capability digest"),
        expires_at_ms: 200,
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

#[test]
fn cutover_effect_authorities_are_closed_and_non_overlapping() {
    assert!(CutoverEffectAuthority::Cutover.permits(CutoverEffectKind::ApplyAdmission));
    assert!(CutoverEffectAuthority::Cutover.permits(CutoverEffectKind::ClosureActivation));
    assert!(!CutoverEffectAuthority::Cutover.permits(CutoverEffectKind::ScopedZoneReset));
    assert!(!CutoverEffectAuthority::Cutover.permits(CutoverEffectKind::DestroyDurableVolume));
    assert!(CutoverEffectAuthority::ResetZone.permits(CutoverEffectKind::ScopedZoneReset));
    assert!(CutoverEffectAuthority::ResetZone.permits(CutoverEffectKind::DestroyDurableVolume));
    assert!(!CutoverEffectAuthority::ResetZone.permits(CutoverEffectKind::HostDrain));
}

#[test]
fn cutover_effect_payload_reuses_opaque_storage_contract() {
    let request = CutoverEffectRequest {
        operation_id: BundleOpId::new("op-payload"),
        authority: CutoverEffectAuthority::Cutover,
        phase: 4,
        effect_id: BundleOpId::new("effect-disposition"),
        effect: CutoverEffectKind::CutoverDisposition,
        replay_class: CutoverReplayClass::Repeatable,
        request_digest: CanonicalAuditDigest::parse("sha256:".to_owned() + &"a".repeat(64))
            .expect("request digest"),
        capability_digest: CanonicalAuditDigest::parse("sha256:".to_owned() + &"b".repeat(64))
            .expect("capability digest"),
        identity: None,
        handoff: None,
        payload: Some(CutoverEffectPayload::Storage(
            ReconcileStorageScopeRequest {
                storage_ref: BundleOpId::new("path:zone-store"),
                apply: true,
                tracing_span_id: None,
            },
        )),
    };
    let encoded = serde_json::to_string(&request).expect("serialize effect");
    assert!(!encoded.contains("/var/"));
    let decoded: CutoverEffectRequest = serde_json::from_str(&encoded).expect("decode effect");
    assert_eq!(decoded, request);

    let admission = CutoverEffectPayload::ApplyAdmission(
        d2b_contracts::broker_wire::CutoverAdmissionRequest {},
    );
    let encoded = serde_json::to_string(&admission).expect("serialize admission");
    let decoded: CutoverEffectPayload = serde_json::from_str(&encoded).expect("decode admission");
    assert_eq!(decoded, admission);
}
