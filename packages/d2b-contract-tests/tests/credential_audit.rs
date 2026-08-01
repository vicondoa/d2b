//! Credential audit and telemetry structural contract.

use d2b_contracts::v3::credential::{CredentialMethod, PlacementBinding};
use d2b_contracts::v3::credential_controller::{
    CREDENTIAL_METRICS, CredentialAuditOutcome, CredentialAuditRecord, CredentialLeaseAggregate,
    CredentialProviderKind, CredentialTelemetryField, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

const FORBIDDEN_LABEL_KEYS: &[&str] = &[
    "vm",
    "zone",
    "zone_id",
    "zone_uid",
    "credential_name",
    "credential_ref",
    "credential_uid",
    "credential_digest",
    "resource_name_digest",
    "d2b.credential.name",
];

#[test]
fn authorized_audit_requires_only_digested_credential_identity() {
    let marker = format!("credential-name-secret-canary-{:x}", std::process::id());
    let subject = format!("subject-secret-canary-{:x}", std::process::id());
    let digest = d2b_contracts::v3::CredentialAuditDigest::after_authorization(marker.as_bytes());
    let subject_digest =
        d2b_contracts::v3::CredentialAuditDigest::after_authorization(subject.as_bytes());
    let record = CredentialAuditRecord::authorized_service(
        true,
        CredentialProviderKind::SecretService,
        "dev",
        subject_digest.as_str(),
        digest.as_str(),
        CredentialMethod::AcquireToken,
        CredentialAuditOutcome::Success,
        1,
        None,
    )
    .unwrap()
    .unwrap();
    let wire = record.to_wire_record();
    assert!(wire.contains("resource_name_digest=sha256:"));
    assert!(wire.contains("authorization_decision=allowed"));
    assert!(wire.contains("role_subresource=use-credential/acquire-token"));
    assert!(!wire.contains(&marker));
    assert!(!wire.contains(&subject));
    assert!(!format!("{record:?}").contains(&marker));
    assert!(
        CredentialAuditRecord::authorized_service(
            true,
            CredentialProviderKind::SecretService,
            "dev",
            subject_digest.as_str(),
            &marker,
            CredentialMethod::AcquireToken,
            CredentialAuditOutcome::Success,
            1,
            None,
        )
        .is_err()
    );
}

#[test]
fn lease_expiry_is_only_a_provider_placement_aggregate() {
    let aggregate = CredentialLeaseAggregate::from_active_expiries(
        CredentialProviderKind::ManagedIdentity,
        PlacementBinding::HostSystem,
        10_000,
        [9_000, 11_500, 15_000],
    )
    .unwrap();
    assert_eq!(aggregate.active_leases, 2);
    assert_eq!(aggregate.minimum_expiry_seconds, 1);
    assert_eq!(
        aggregate
            .labels()
            .into_iter()
            .map(|field| field.key)
            .collect::<Vec<_>>(),
        ["provider", "placement_binding"]
    );
}

#[test]
fn denied_requests_emit_no_identity_bearing_audit_record() {
    let marker = format!("credential-name-secret-canary-{:x}", std::process::id());
    let denied = CredentialAuditRecord::authorized_service(
        false,
        CredentialProviderKind::Entra,
        &marker,
        &marker,
        &marker,
        CredentialMethod::RefreshToken,
        CredentialAuditOutcome::Denied,
        1,
        Some(marker.clone()),
    )
    .unwrap();
    assert!(denied.is_none());
}

#[test]
fn metric_descriptor_keys_exclude_every_credential_identity_class() {
    let expected = [
        descriptor(
            "d2b_credential_operations_total",
            &[
                "provider",
                "operation_class",
                "placement_binding",
                "outcome",
            ],
        ),
        descriptor(
            "d2b_credential_lease_expiry_seconds",
            &["provider", "placement_binding"],
        ),
        descriptor(
            "d2b_credential_rotation_total",
            &["provider", "policy", "outcome"],
        ),
        descriptor("d2b_credential_provider_health", &["provider"]),
        descriptor(
            "d2b_credential_active_leases",
            &["provider", "placement_binding"],
        ),
    ];
    assert_eq!(CREDENTIAL_METRICS.len(), expected.len());
    for (actual, expected) in CREDENTIAL_METRICS.iter().zip(expected) {
        assert_eq!(actual.name, expected.0);
        assert_eq!(actual.label_keys, expected.1);
        for key in actual.label_keys {
            assert!(!FORBIDDEN_LABEL_KEYS.contains(key));
            assert!(!key.contains("resource_name"));
        }
    }
}

#[test]
fn complete_frames_pass_but_identity_keys_or_values_reject_the_whole_frame() {
    let zone_canary = format!("zone-canary-{:x}", std::process::id());
    for provider in [
        CredentialProviderKind::SecretService,
        CredentialProviderKind::Entra,
        CredentialProviderKind::ManagedIdentity,
    ] {
        let frame = CredentialTelemetryFrame::new(
            provider,
            &zone_canary,
            CredentialTelemetryOperation::Reconcile,
            CredentialTelemetryOutcome::Success,
            PlacementBinding::GuestAgent,
            7,
            "1.0.0",
        )
        .unwrap();
        assert!(CredentialTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok());
        assert!(
            frame
                .resource_attributes()
                .iter()
                .any(|field| { field.key == "d2b.zone" && field.value == zone_canary })
        );
        assert!(
            frame
                .resource_attributes()
                .iter()
                .any(|field| field.key == "d2b.provider")
        );
        assert!(
            frame
                .resource_attributes()
                .iter()
                .any(|field| field.key == "d2b.component")
        );
        assert!(
            frame
                .resource_attributes()
                .iter()
                .any(|field| field.key == "service.name")
        );
        assert!(
            frame
                .span_attributes()
                .iter()
                .chain(frame.metric_labels())
                .all(|field| field.value != zone_canary)
        );
    }

    let marker = format!("entra-token-canary-{:x}", std::process::id());
    for field in [
        CredentialTelemetryField {
            key: "d2b.credential.name",
            value: "bounded".to_owned(),
        },
        CredentialTelemetryField {
            key: "outcome",
            value: marker,
        },
        CredentialTelemetryField {
            key: "provider",
            value: "credential-name-derived".to_owned(),
        },
        CredentialTelemetryField {
            key: "outcome",
            value: "arbitrary".to_owned(),
        },
    ] {
        assert!(CredentialTelemetryFrame::validate_collector_fields([field]).is_err());
    }
}

fn descriptor(
    name: &'static str,
    label_keys: &'static [&'static str],
) -> (&'static str, &'static [&'static str]) {
    (name, label_keys)
}
