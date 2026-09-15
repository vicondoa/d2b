#[cfg(not(feature = "layer1-bootstrap"))]
#[path = "common/mod.rs"]
mod common;

#[cfg(not(feature = "layer1-bootstrap"))]
use std::os::fd::AsRawFd;

#[cfg(not(feature = "layer1-bootstrap"))]
use common::TestBroker;
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_broker::protocol::{connect_seqpacket, recv_json_frame, send_json_frame};
use d2b_contracts::types::{BundleOpId, ScopeId};
use d2b_contracts_broker::PROTOCOL_VERSION;
use d2b_contracts_broker::broker_wire::{
    ApplyNftablesProjectionRequest, BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope,
    BrokerResponse, CreateBridgeRequest, DeleteBridgeRequest, DeletePersistentTapRequest,
    NftablesProjectionAction,
};
use d2b_contracts_resource::v3::{ResourceBundleGenerationId, ResourceGeneration, ResourceUid};
use serde::{Deserialize, Serialize};

const PREVIOUS_PROTOCOL_VERSION: u32 = 3;

/// The process-family wire variants U10 retired at wire v6 (KTD10). The
/// matrix pins them exactly: each is still the *literal frame shape* an
/// old binary at wire v<6 sent, none of them decodes as a current
/// `RequestEnvelope`, and the retired-wire gate names every one of them
/// with the v6 boundary, the typed stale-wire-version refusal, and an
/// audit record.
const RETIRED_PROCESS_VARIANTS: &[&str] = &[
    "OpenPidfd",
    "OpenPeerPidfdFromAcceptedSocket",
    "ObserveRunner",
    "PollChildReaped",
    "PrepareRuntimeDir",
    "PrepareStateDir",
    "CgroupKill",
    "SignalRunner",
    "DeregisterRunnerPidfd",
    "SpawnRunner",
];

// The production envelope carries no protocol version and has no negotiation
// path. These reduced prior-version types pin the actual serde compatibility:
// an old stable request remains readable, a request this protocol retired is
// unknown to the current decoder, and each new operation is unknown to an old
// decoder.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
enum PreviousBrokerRequest {
    Hello {
        #[serde(rename = "clientVersion")]
        client_version: String,
    },
    ValidateBundle,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviousBrokerRequestEnvelope {
    request: PreviousBrokerRequest,
}

fn current_envelope(request: BrokerRequest) -> BrokerRequestEnvelope {
    BrokerRequestEnvelope {
        request,
        caller_role: BrokerCallerRole::NotAuthorized,
        test_peer_uid: None,
        audit_join: None,
    }
}

fn current_only_requests() -> [BrokerRequest; 4] {
    let generation_id = ResourceBundleGenerationId::parse(format!("sha256:{}", "1".repeat(64)))
        .expect("valid generation identity");
    [
        BrokerRequest::ApplyNftablesProjection(ApplyNftablesProjectionRequest {
            bundle_nft_projection_intent_ref: BundleOpId::new("nft-projection:test"),
            scope_id: ScopeId::new("scope:test"),
            action: NftablesProjectionAction::Apply,
            zone_uid: ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            network_uid: ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            network_generation: ResourceGeneration::new(7).unwrap(),
            attachment_generation: ResourceGeneration::new(11).unwrap(),
            expected_generation_id: generation_id,
            desired_hash: None,
            tracing_span_id: None,
        }),
        BrokerRequest::CreateBridge(CreateBridgeRequest {
            bundle_bridge_intent_ref: BundleOpId::new("bridge:test"),
            scope_id: ScopeId::new("scope:test"),
            zone_uid: ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            network_uid: ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            network_generation: ResourceGeneration::new(7).unwrap(),
            attachment_generation: ResourceGeneration::new(11).unwrap(),
            bundle_generation: ResourceBundleGenerationId::parse(format!(
                "sha256:{}",
                "1".repeat(64)
            ))
            .unwrap(),
            tracing_span_id: None,
        }),
        BrokerRequest::DeleteBridge(DeleteBridgeRequest {
            bundle_bridge_intent_ref: BundleOpId::new("bridge:test"),
            scope_id: ScopeId::new("scope:test"),
            zone_uid: ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            network_uid: ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            network_generation: ResourceGeneration::new(7).unwrap(),
            attachment_generation: ResourceGeneration::new(11).unwrap(),
            bundle_generation: ResourceBundleGenerationId::parse(format!(
                "sha256:{}",
                "1".repeat(64)
            ))
            .unwrap(),
            tracing_span_id: None,
        }),
        BrokerRequest::DeletePersistentTap(DeletePersistentTapRequest {
            attachment_id: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("valid attachment id"),
            expected_zone_uid: ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
                .expect("valid zone id"),
            expected_network_uid: ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002")
                .expect("valid network id"),
            expected_network_generation: ResourceGeneration::new(7)
                .expect("valid network generation"),
            expected_attachment_generation: ResourceGeneration::new(11)
                .expect("valid attachment generation"),
            expected_bundle_generation: ResourceBundleGenerationId::parse(
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .expect("valid bundle generation"),
            tracing_span_id: None,
        }),
    ]
}

#[test]
fn previous_client_request_decodes_under_current_protocol() {
    assert_eq!(PREVIOUS_PROTOCOL_VERSION, 3);
    assert_eq!(PROTOCOL_VERSION, 6);

    let encoded = serde_json::to_vec(&PreviousBrokerRequestEnvelope {
        request: PreviousBrokerRequest::Hello {
            client_version: "0.0.0-test".to_owned(),
        },
    })
    .expect("previous request serializes");
    let decoded: BrokerRequestEnvelope =
        serde_json::from_slice(&encoded).expect("current broker decodes previous request");

    assert!(matches!(decoded.request, BrokerRequest::Hello(_)));
}

#[test]
fn a_retired_previous_request_is_unknown_to_the_current_decoder() {
    // The previous protocol carried `ValidateBundle` and this one retired it
    // with its row, so a client that still sends it is refused as an unknown
    // variant rather than served.
    let encoded = serde_json::to_vec(&PreviousBrokerRequestEnvelope {
        request: PreviousBrokerRequest::ValidateBundle,
    })
    .expect("previous request serializes");
    let error = serde_json::from_slice::<BrokerRequestEnvelope>(&encoded)
        .expect_err("the current broker must not decode a retired request");
    assert!(
        error.to_string().contains("unknown variant"),
        "a retired request failed for an unexpected reason: {error}"
    );
}

#[test]
fn the_retired_wire_gate_machinery_names_a_retired_variant() {
    use d2b_broker::runtime::{RetiredWireVariant, retired_wire_variant};
    // The machinery U10's retirements inherit: a retired variant is
    // recognized by name and carries the negotiated-wire boundary it was
    // retired at. `ValidateBundle` is the previous protocol's request the
    // current protocol retired (see
    // `a_retired_previous_request_is_unknown_to_the_current_decoder` above),
    // standing in as the fixture retired-variant of the old/new matrix.
    const FIXTURE: &[RetiredWireVariant] = &[RetiredWireVariant {
        variant: "ValidateBundle",
        retired_in_version: 4,
    }];
    let retired = retired_wire_variant("ValidateBundle", FIXTURE)
        .expect("the fixture table names the retired variant");
    assert_eq!(retired.retired_in_version, 4);
    // A variant the table does not carry - current or never-existing - is
    // not gated.
    assert!(retired_wire_variant("Hello", FIXTURE).is_none());
    // The boundary sits between the previous protocol and the current one:
    // a straggler negotiated before the boundary, and the gate - not the
    // decoder - is what answers its call with the stale-wire-version
    // refusal.
    assert!(PREVIOUS_PROTOCOL_VERSION < retired.retired_in_version);
    assert!(PROTOCOL_VERSION > retired.retired_in_version);
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn the_retired_wire_gate_names_every_retired_process_variant_at_wire_v6() {
    use d2b_broker::catalog::WIRE_VARIANTS;
    use d2b_broker::runtime::{RETIRED_WIRE_VARIANTS, retired_wire_variant};

    assert_eq!(PROTOCOL_VERSION, 6);
    // The production table names exactly the ten process-family variants
    // the cut retired, all at the same wire boundary.
    let mut names: Vec<&str> = RETIRED_WIRE_VARIANTS.iter().map(|entry| entry.variant).collect();
    names.sort_unstable();
    let mut expected = RETIRED_PROCESS_VARIANTS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected, "the retirement table and the matrix agree");
    for variant in RETIRED_PROCESS_VARIANTS {
        let retired = retired_wire_variant(variant, RETIRED_WIRE_VARIANTS)
            .expect("the gate names every retired process variant");
        assert_eq!(retired.retired_in_version, PROTOCOL_VERSION);
        assert!(
            PREVIOUS_PROTOCOL_VERSION < retired.retired_in_version,
            "{variant} was retired before the previous protocol boundary"
        );
    }
    // The wire enum no longer declares any retired variant: the current
    // decoder cannot spell a retired `kind`, and the gate alone still
    // recognizes the name as a straggler it must refuse.
    for variant in RETIRED_PROCESS_VARIANTS {
        assert!(
            WIRE_VARIANTS.iter().all(|declared| *declared != *variant),
            "{variant} is retired but the wire enum still declares it"
        );
    }
    // A current wire variant is not gated.
    assert!(retired_wire_variant("Hello", RETIRED_WIRE_VARIANTS).is_none());
    assert!(retired_wire_variant("EnvelopeInvoke", RETIRED_WIRE_VARIANTS).is_none());
}

#[test]
fn a_retired_process_variant_old_frame_is_unknown_to_the_current_decoder() {
    // A literal wire-v<6 frame for each retired process variant: the
    // current decoder must refuse every one as an unknown variant (the arm
    // is gone), never decode it into a neighboring request.
    for variant in RETIRED_PROCESS_VARIANTS {
        let encoded = serde_json::to_vec(&serde_json::json!({
            "request": { "kind": variant, "payload": {} },
        }))
        .expect("the old-binary frame serializes");
        let error = serde_json::from_slice::<BrokerRequestEnvelope>(&encoded)
            .expect_err("the current broker must not decode a retired variant");
        assert!(
            error.to_string().contains("unknown variant"),
            "{variant} failed for an unexpected reason: {error}"
        );
    }
}

#[test]
#[cfg(not(feature = "layer1-bootstrap"))]
fn an_old_binary_retired_process_variant_frame_is_refused_with_the_stale_wire_code_and_audited() {
    // The full mixed-version matrix on the real broker binary (KTD10):
    // each literal v<6 process-family frame is answered with the typed
    // stale-wire-version refusal that names the variant and the v6
    // boundary, and each refusal produces an audit record under the
    // variant's own operation name - the record vocabulary is the
    // pre-retirement op name, unchanged across the cut.
    let broker = TestBroker::spawn("retired-wire-gate-");
    for variant in RETIRED_PROCESS_VARIANTS {
        let client = connect_seqpacket(broker.socket_path()).expect("connect broker");
        send_json_frame(
            client.as_raw_fd(),
            &serde_json::json!({
                "request": { "kind": variant, "payload": {} },
                "callerRole": { "role": "AdminUid", "uid": 0 },
            }),
        )
        .expect("send the old-binary frame");
        let response: BrokerResponse = recv_json_frame(client.as_raw_fd())
            .expect("receive the gate reply")
            .expect("the gate wrote a reply frame");
        let BrokerResponse::Error(refusal) = response else {
            panic!("expected a typed stale-wire-version refusal, got {response:?}");
        };
        assert_eq!(refusal.kind, d2b_broker::envelope::STALE_WIRE_VERSION);
        assert_eq!(refusal.operation, *variant);
        assert!(
            refusal.message.contains(variant) && refusal.message.contains("6"),
            "the refusal names the retired variant and the wire boundary it was retired at: {}",
            refusal.message
        );
    }

    let audit = broker.audit_contents();
    for variant in RETIRED_PROCESS_VARIANTS {
        assert!(
            audit.contains(&format!(r#""op":"{variant}""#)),
            "the audit record carries the committed op name {variant}: {audit}"
        );
    }
    assert_eq!(
        audit.matches(r#""disposition":"stale-wire-version""#).count(),
        RETIRED_PROCESS_VARIANTS.len(),
        "every retired call is audited with the stale-wire-version disposition: {audit}"
    );
    assert_eq!(
        audit.matches(r#""outcome":"refused""#).count(),
        RETIRED_PROCESS_VARIANTS.len(),
        "every retired call is audited as refused: {audit}"
    );
}

#[test]
fn current_only_requests_are_rejected_by_previous_decoder() {
    assert_eq!(PREVIOUS_PROTOCOL_VERSION, 3);
    assert_eq!(PROTOCOL_VERSION, 6);

    for request in current_only_requests() {
        let operation = request.op_name();
        let encoded =
            serde_json::to_vec(&current_envelope(request)).expect("current request serializes");
        let error = serde_json::from_slice::<PreviousBrokerRequestEnvelope>(&encoded)
            .expect_err("previous broker must reject a current-only operation");
        assert!(
            error.to_string().contains("unknown variant"),
            "{operation} failed for an unexpected reason: {error}"
        );
    }
}
