use d2b_contracts::v3::credential::{
    AudienceToken, CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion,
    OperationClass,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_credential_service::{
    CredentialAuthorization, CredentialMetadata, CredentialMethod, CredentialOutcomeCode,
    CredentialRequest, CredentialServiceError, CredentialServiceErrorCode, DeliveryResponse,
    DeliveryRouteDigest, DeliverySessionParams, SensitiveDeliveryRecord, encode_outer,
};

#[test]
fn process_unique_secret_canary_never_reaches_outer_or_diagnostic_surfaces() {
    let nonce = format!("{:x}", std::process::id());
    let secret = format!("secret-canary-{nonce}");
    let lease = CredentialLeaseHandle::parse(&secret).unwrap();
    let source = CredentialSourceVersion::parse(&secret).unwrap();
    let request = CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        format!("operation-{nonce}"),
        format!("idempotency-{nonce}"),
        2_000,
        1_500,
    )
    .unwrap();
    let delivery_session_params = DeliverySessionParams::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/display-wayland").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        AudienceToken::parse("azure-resource-manager").unwrap(),
        OperationClass::AcquireToken,
        2_000,
        1_500,
        DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        4_096,
        1,
    )
    .unwrap();
    let authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery_session_params.clone()),
    )
    .unwrap();
    let response = DeliveryResponse {
        metadata: CredentialMetadata {
            lease_handle: lease,
            rotation_generation: 1,
            source_version: source,
            expires_at_unix_ms: 2_000,
            state: CredentialLeaseState::Active,
            outcome: CredentialOutcomeCode::Success,
        },
        delivery_session_params,
    };
    let record = SensitiveDeliveryRecord::new(secret.as_bytes().to_vec(), 4_096).unwrap();
    let error = CredentialServiceError::new(CredentialServiceErrorCode::ProviderUnavailable);
    let surfaces = [
        format!("{request:?}"),
        format!("{authorization:?}"),
        format!("{response:?}"),
        String::from_utf8_lossy(&encode_outer(&response).unwrap()).into_owned(),
        format!("{record:?}"),
        format!("{error:?}"),
        error.to_string(),
    ];
    for surface in surfaces {
        assert!(
            !surface.contains(&secret),
            "secret canary reached a rendered surface"
        );
    }
    let mut copied = vec![0; secret.len()];
    record.copy_to(&mut copied).unwrap();
    assert_eq!(copied, secret.as_bytes());
    copied.fill(0);
}

#[test]
fn sensitive_delivery_record_explicitly_zeroizes_before_reuse() {
    let mut record = SensitiveDeliveryRecord::new(b"plaintext-secret".to_vec(), 128).unwrap();
    let mut copied = vec![0; 16];
    record.copy_to(&mut copied).unwrap();
    assert_eq!(copied, b"plaintext-secret");
    copied.fill(0);
    record.clear();
    assert!(record.is_cleared());
    assert!(record.is_zeroized());
    assert!(record.copy_to(&mut copied).is_err());
}
