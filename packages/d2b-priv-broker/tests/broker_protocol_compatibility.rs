use d2b_contracts::PROTOCOL_VERSION;
use d2b_contracts::broker_wire::{
    ApplyNftablesProjectionRequest, BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope,
    CreateBridgeRequest, DeleteBridgeRequest, DeletePersistentTapRequest, NftablesProjectionAction,
};
use d2b_contracts::types::{BundleOpId, RoleId, ScopeId, VmId};
use serde::{Deserialize, Serialize};

const PREVIOUS_PROTOCOL_VERSION: u32 = 3;

// The production envelope carries no protocol version and has no negotiation
// path. These reduced prior-version types pin the actual serde compatibility:
// an old stable request remains readable, while each new operation is unknown
// to an old decoder.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
enum PreviousBrokerRequest {
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
    }
}

fn current_only_requests() -> [BrokerRequest; 4] {
    [
        BrokerRequest::ApplyNftablesProjection(ApplyNftablesProjectionRequest {
            bundle_nft_projection_intent_ref: BundleOpId::new("nft-projection:test"),
            scope_id: ScopeId::new("scope:test"),
            action: NftablesProjectionAction::Apply,
            desired_hash: None,
            tracing_span_id: None,
        }),
        BrokerRequest::CreateBridge(CreateBridgeRequest {
            bundle_bridge_intent_ref: BundleOpId::new("bridge:test"),
            scope_id: ScopeId::new("scope:test"),
            tracing_span_id: None,
        }),
        BrokerRequest::DeleteBridge(DeleteBridgeRequest {
            bundle_bridge_intent_ref: BundleOpId::new("bridge:test"),
            scope_id: ScopeId::new("scope:test"),
            tracing_span_id: None,
        }),
        BrokerRequest::DeletePersistentTap(DeletePersistentTapRequest {
            role_id: RoleId::new("role:test"),
            vm_id: VmId::new("vm:test"),
            tracing_span_id: None,
        }),
    ]
}

#[test]
fn previous_client_request_decodes_under_current_protocol() {
    assert_eq!(PREVIOUS_PROTOCOL_VERSION, 3);
    assert_eq!(PROTOCOL_VERSION, 4);

    let encoded = serde_json::to_vec(&PreviousBrokerRequestEnvelope {
        request: PreviousBrokerRequest::ValidateBundle,
    })
    .expect("previous request serializes");
    let decoded: BrokerRequestEnvelope =
        serde_json::from_slice(&encoded).expect("current broker decodes previous request");

    assert!(matches!(decoded.request, BrokerRequest::ValidateBundle));
}

#[test]
fn current_only_requests_are_rejected_by_previous_decoder() {
    assert_eq!(PREVIOUS_PROTOCOL_VERSION, 3);
    assert_eq!(PROTOCOL_VERSION, 4);

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
