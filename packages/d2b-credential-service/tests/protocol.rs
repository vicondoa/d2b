use d2b_contracts::v3::credential::{
    AudienceToken, CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion,
    OperationClass,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_credential_service::{
    CredentialMetadata, CredentialMethod, CredentialOutcomeCode, CredentialRequest,
    CredentialResponse, DeliveryResponse, DeliveryRouteDigest, DeliverySessionParams,
    MAX_CREDENTIAL_MESSAGE_BYTES, MetadataResponse, decode_outer, encode_outer,
};

const CREDENTIAL_PROTO: &str = include_str!("../../d2b-contracts/proto/v3/credential.proto");

fn request() -> CredentialRequest {
    CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        "operation-1",
        "idempotency-1",
        2_000,
        1_500,
    )
    .unwrap()
}

fn metadata() -> CredentialMetadata {
    metadata_for(CredentialLeaseState::Active, CredentialOutcomeCode::Success)
}

fn metadata_for(state: CredentialLeaseState, outcome: CredentialOutcomeCode) -> CredentialMetadata {
    CredentialMetadata {
        lease_handle: CredentialLeaseHandle::parse("lease-1").unwrap(),
        rotation_generation: 3,
        source_version: CredentialSourceVersion::parse("source-1").unwrap(),
        expires_at_unix_ms: 2_000,
        state,
        outcome,
    }
}

fn delivery(operation_class: OperationClass) -> DeliverySessionParams {
    DeliverySessionParams::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(4).unwrap(),
        ResourceRef::parse("Provider/display-wayland").unwrap(),
        ResourceGeneration::new(5).unwrap(),
        AudienceToken::parse("azure-resource-manager").unwrap(),
        operation_class,
        2_000,
        1_500,
        DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        4_096,
        7,
    )
    .unwrap()
}

fn push_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn push_string(output: &mut Vec<u8>, field: u8, value: &str) {
    output.push((field << 3) | 2);
    push_varint(output, value.len() as u64);
    output.extend_from_slice(value.as_bytes());
}

fn push_u64(output: &mut Vec<u8>, field: u8, value: u64) {
    output.push(field << 3);
    push_varint(output, value);
}

fn push_message(output: &mut Vec<u8>, field: u8, value: &[u8]) {
    output.push((field << 3) | 2);
    push_varint(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn metadata_vector(state_code: u64, outcome_code: u64) -> Vec<u8> {
    let value = metadata();
    let mut expected = Vec::new();
    push_string(&mut expected, 1, value.lease_handle.as_opaque_str());
    push_u64(&mut expected, 2, 3);
    push_string(&mut expected, 3, value.source_version.as_opaque_str());
    push_u64(&mut expected, 4, 2_000);
    push_u64(&mut expected, 5, state_code);
    push_u64(&mut expected, 6, outcome_code);
    expected
}

fn delivery_vector(operation_code: u64) -> Vec<u8> {
    let mut expected = Vec::new();
    push_string(&mut expected, 1, "Credential/work-entra");
    push_string(&mut expected, 2, "123e4567-e89b-42d3-a456-426614174000");
    push_u64(&mut expected, 3, 4);
    push_string(&mut expected, 4, "Provider/display-wayland");
    push_u64(&mut expected, 5, 5);
    push_string(&mut expected, 6, "azure-resource-manager");
    push_u64(&mut expected, 7, operation_code);
    push_u64(&mut expected, 8, 2_000);
    push_u64(&mut expected, 9, 1_500);
    push_string(&mut expected, 10, &format!("sha256:{}", "a".repeat(64)));
    push_u64(&mut expected, 11, 1);
    push_u64(&mut expected, 12, 4_096);
    push_u64(&mut expected, 13, 7);
    expected
}

fn delivery_response_vector(operation_code: u64) -> Vec<u8> {
    let mut expected = Vec::new();
    push_message(&mut expected, 1, &metadata_vector(1, 1));
    push_message(&mut expected, 2, &delivery_vector(operation_code));
    expected
}

fn metadata_response_vector(state_code: u64, outcome_code: u64) -> Vec<u8> {
    let mut expected = Vec::new();
    push_message(&mut expected, 1, &metadata_vector(state_code, outcome_code));
    expected
}

#[test]
fn proto_pins_the_exact_service_and_request_shape() {
    let methods = CREDENTIAL_PROTO
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("rpc "))
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "rpc AcquireToken(CredentialRequest) returns (AcquireTokenResponse);",
            "rpc RefreshToken(CredentialRequest) returns (RefreshTokenResponse);",
            "rpc RevokeToken(CredentialRequest) returns (RevokeTokenResponse);",
            "rpc SignChallenge(CredentialRequest) returns (SignChallengeResponse);",
            "rpc InspectMetadata(CredentialRequest) returns (InspectMetadataResponse);",
        ]
    );
    let request = CREDENTIAL_PROTO
        .split_once("message CredentialRequest {")
        .unwrap()
        .1
        .split_once('}')
        .unwrap()
        .0;
    assert!(request.contains("string credential_ref = 1;"));
    assert!(request.contains("string operation_id = 2;"));
    assert!(request.contains("string idempotency_key = 3;"));
    assert!(request.contains("uint64 requested_expiry_unix_ms = 4;"));
    assert!(request.contains("uint64 deadline_unix_ms = 5;"));
    assert!(!request.contains("operation_class"));
}

#[test]
fn all_five_method_vectors_round_trip_with_exact_response_shapes() {
    let request_vector = b"\x0a\x15Credential/work-entra\x12\x0boperation-1\x1a\x0didempotency-1\x20\xd0\x0f\x28\xdc\x0b";
    assert_eq!(encode_outer(&request()).unwrap(), request_vector);
    assert_eq!(
        decode_outer::<CredentialRequest>(request_vector).unwrap(),
        request()
    );

    let responses = [
        CredentialResponse::AcquireToken(DeliveryResponse {
            metadata: metadata(),
            delivery_session_params: delivery(OperationClass::AcquireToken),
        }),
        CredentialResponse::RefreshToken(DeliveryResponse {
            metadata: metadata(),
            delivery_session_params: delivery(OperationClass::RefreshToken),
        }),
        CredentialResponse::RevokeToken(MetadataResponse {
            metadata: metadata_for(
                CredentialLeaseState::Revoked,
                CredentialOutcomeCode::Revoked,
            ),
        }),
        CredentialResponse::SignChallenge(DeliveryResponse {
            metadata: metadata(),
            delivery_session_params: delivery(OperationClass::SignChallenge),
        }),
        CredentialResponse::InspectMetadata(MetadataResponse {
            metadata: metadata(),
        }),
    ];
    let methods = [
        CredentialMethod::AcquireToken,
        CredentialMethod::RefreshToken,
        CredentialMethod::RevokeToken,
        CredentialMethod::SignChallenge,
        CredentialMethod::InspectMetadata,
    ];
    for (response, method) in responses.into_iter().zip(methods) {
        let bytes = match &response {
            CredentialResponse::AcquireToken(value)
            | CredentialResponse::RefreshToken(value)
            | CredentialResponse::SignChallenge(value) => encode_outer(value).unwrap(),
            CredentialResponse::RevokeToken(value) | CredentialResponse::InspectMetadata(value) => {
                encode_outer(value).unwrap()
            }
        };
        let expected = match method {
            CredentialMethod::AcquireToken => delivery_response_vector(1),
            CredentialMethod::RefreshToken => delivery_response_vector(2),
            CredentialMethod::RevokeToken => metadata_response_vector(3, 2),
            CredentialMethod::SignChallenge => delivery_response_vector(4),
            CredentialMethod::InspectMetadata => metadata_response_vector(1, 1),
        };
        assert_eq!(bytes, expected);
        match &response {
            CredentialResponse::AcquireToken(value)
            | CredentialResponse::RefreshToken(value)
            | CredentialResponse::SignChallenge(value) => {
                assert_eq!(decode_outer::<DeliveryResponse>(&bytes).unwrap(), *value);
            }
            CredentialResponse::RevokeToken(value) | CredentialResponse::InspectMetadata(value) => {
                assert_eq!(decode_outer::<MetadataResponse>(&bytes).unwrap(), *value);
            }
        }
        assert_eq!(response.method(), method);
        assert_eq!(
            response.delivery_session_params().is_some(),
            method.requires_delivery()
        );
    }
}

#[test]
fn malformed_unknown_and_oversize_messages_fail_closed() {
    let mut unknown = encode_outer(&request()).unwrap();
    unknown.extend_from_slice(&[0x30, 0x01]);
    assert!(decode_outer::<CredentialRequest>(&unknown).is_err());
    let duplicate = [encode_outer(&request()).unwrap(), vec![0x20, 0xd0, 0x0f]].concat();
    assert!(decode_outer::<CredentialRequest>(&duplicate).is_err());
    let noncanonical_key = [
        vec![0x8a, 0x00],
        encode_outer(&request()).unwrap()[1..].to_vec(),
    ]
    .concat();
    assert!(decode_outer::<CredentialRequest>(&noncanonical_key).is_err());
    assert!(
        decode_outer::<CredentialRequest>(&vec![b'x'; MAX_CREDENTIAL_MESSAGE_BYTES + 1]).is_err()
    );
    assert!(
        CredentialRequest::new(
            ResourceRef::parse("Provider/not-a-credential").unwrap(),
            "operation-1",
            "idempotency-1",
            2_000,
            1_500,
        )
        .is_err()
    );
    assert!(
        CredentialRequest::new(
            ResourceRef::parse("Credential/work-entra").unwrap(),
            "operation-1",
            "idempotency-1",
            1_000,
            1_001,
        )
        .is_err()
    );
}

#[test]
fn delivery_binding_round_trips_and_rejects_non_delivery_operations() {
    let params = delivery(OperationClass::AcquireToken);
    let bytes = encode_outer(&params).unwrap();
    assert_eq!(
        decode_outer::<DeliverySessionParams>(&bytes).unwrap(),
        params
    );
    assert_eq!(params.sequence(), 7);
    assert!(
        DeliverySessionParams::new(
            ResourceRef::parse("Credential/work-entra").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(4).unwrap(),
            ResourceRef::parse("Provider/display-wayland").unwrap(),
            ResourceGeneration::new(5).unwrap(),
            AudienceToken::parse("azure-resource-manager").unwrap(),
            OperationClass::InspectMetadata,
            2_000,
            1_500,
            DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            4_096,
            7,
        )
        .is_err()
    );
}
