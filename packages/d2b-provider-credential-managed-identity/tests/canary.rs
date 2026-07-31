mod common;

use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseStatus, CredentialStatus, PlacementBinding,
};
use d2b_credential_service::{
    CredentialMethod, CredentialResponse, CredentialServer, CredentialServiceError,
    CredentialServiceErrorCode, CredentialTransport, encode_outer,
};
use d2b_provider_credential_managed_identity::{
    ManagedIdentityAuditOperation, ManagedIdentityAuditOutcome, ManagedIdentityAuditRecord,
    ManagedIdentityTelemetryFrame, ManagedIdentityTelemetryOperation,
    ManagedIdentityTelemetryOutcome, TelemetryField,
};

use common::{admitted, request, setup};

#[test]
fn process_unique_managed_identity_canaries_are_absent_from_rendered_surfaces() {
    let nonce = format!("{:x}", std::process::id());
    let credential_name = format!("credential-name-{nonce}");
    let credential_ref = format!("Credential/{credential_name}");
    let credential_uid = format!("credential-uid-{nonce}");
    let credential_digest = format!("credential-digest-{nonce}");
    let (provider, client) = setup();
    let provider_debug = format!("{provider:?}");
    let config_debug = format!("{:?}", provider.config());
    let server = CredentialServer::new(provider, admitted());
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-canary"))
        .unwrap();
    let CredentialResponse::AcquireToken(delivery) = &response else {
        panic!("acquire response");
    };
    let status = CredentialStatus::new(
        CredentialInteractionState::NotRequired,
        None,
        None,
        Some(
            CredentialLeaseStatus::new(
                delivery.metadata.lease_handle.clone(),
                delivery.metadata.state,
                delivery.metadata.rotation_generation,
                delivery.metadata.source_version.clone(),
                delivery.metadata.expires_at_unix_ms,
                1,
                None,
                None,
                PlacementBinding::GuestAgent,
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let error = CredentialServiceError::new(CredentialServiceErrorCode::ProviderUnavailable);
    let authorized_audit = format!(
        "provider=credential-managed-identity operation=acquire-token resource_name_digest=sha256:{} outcome=success",
        "e".repeat(64)
    );
    let typed_audit = ManagedIdentityAuditRecord::new(
        format!("sha256:{}", "e".repeat(64)),
        ManagedIdentityAuditOperation::AcquireToken,
        ManagedIdentityAuditOutcome::Success,
        1,
    )
    .unwrap();
    let telemetry = ManagedIdentityTelemetryFrame::new(
        "dev",
        ManagedIdentityTelemetryOperation::AcquireToken,
        ManagedIdentityTelemetryOutcome::Success,
        PlacementBinding::GuestAgent,
    );
    assert!(
        ManagedIdentityTelemetryFrame::validate_collector_fields(telemetry.all_fields()).is_ok()
    );
    assert!(
        ManagedIdentityTelemetryFrame::validate_collector_fields([TelemetryField {
            key: "d2b.credential.name",
            value: credential_name.clone(),
        }])
        .is_err()
    );
    let surfaces = [
        provider_debug,
        config_debug,
        format!("{response:?}"),
        String::from_utf8_lossy(&encode_outer(delivery).unwrap()).into_owned(),
        format!("{status:?}"),
        serde_json::to_string(&status).unwrap(),
        format!("{error:?}"),
        error.to_string(),
        authorized_audit,
        format!("{typed_audit:?}"),
        typed_audit.to_wire_record(),
        format!("{telemetry:?}"),
        format!("{:?}", telemetry.resource_attributes()),
        format!("{:?}", telemetry.span_attributes()),
        format!("{:?}", telemetry.metric_labels()),
        "provider=credential-managed-identity operation=acquire-token outcome=success".to_owned(),
        "d2b.credential.provider=credential-managed-identity operation_class=acquire-token outcome=success"
            .to_owned(),
    ];
    let markers = [
        client.token_canary.as_str(),
        client.endpoint_canary.as_str(),
        client.response_canary.as_str(),
        provider_client_id_marker(),
        credential_name.as_str(),
        credential_ref.as_str(),
        credential_uid.as_str(),
        credential_digest.as_str(),
    ];
    for surface in surfaces {
        for marker in markers {
            assert!(
                !surface.contains(marker),
                "managed identity canary reached a rendered surface"
            );
        }
    }
}

fn provider_client_id_marker() -> &'static str {
    "client-1234"
}
