use d2b_contracts_resource::v3::{CanonicalJsonValue, ResourceRef, ResourceUid, SchemaFingerprint};
use d2b_provider_runtime_cloud_hypervisor::{
    GuestLocalError, GuestLocalSeedBatch, GuestLocalSeedResource, GuestLocalSeedResourceError,
};

const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const DESCRIPTOR_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

fn guest_ref() -> ResourceRef {
    ResourceRef::parse("Guest/gateway").unwrap()
}

fn payload(name: &str, kind: &str, extra: &str) -> Vec<u8> {
    let spec = if extra.is_empty() {
        r#""executionRef":"Guest/gateway""#
    } else {
        extra
    };
    let raw = format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"createdAt":"2026-08-29T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"controller","name":"{name}","ownerRef":"Guest/gateway","revision":1,"updatedAt":"2026-08-29T00:00:00.000Z","zone":"work"}},"spec":{{{spec}}},"status":{{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{{}},"update":{{"dependencies":{{"count":0,"refs":[]}},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{{"count":0,"refs":[]}},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}},"type":"{kind}"}}"#
    )
    .into_bytes();
    CanonicalJsonValue::parse(&raw)
        .unwrap()
        .to_canonical_bytes()
}

fn resource(name: &str, kind: &str, extra: &str) -> GuestLocalSeedResource {
    GuestLocalSeedResource::new(
        ResourceRef::parse(&format!("{kind}/{name}")).unwrap(),
        guest_ref(),
        payload(name, kind, extra),
    )
    .unwrap()
}

#[test]
fn seed_batch_is_complete_name_addressed_and_uid_free() {
    let batch = GuestLocalSeedBatch::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        "seed-operation",
        vec![
            resource("gateway-agent", "Process", ""),
            resource("gateway-activation", "EphemeralProcess", ""),
        ],
    )
    .unwrap();
    assert_eq!(batch.resources().len(), 2);
    assert!(batch.resources()[0].resource_ref() < batch.resources()[1].resource_ref());
    assert!(
        batch
            .resources()
            .iter()
            .all(|seed| { !String::from_utf8_lossy(seed.canonical_json()).contains("\"uid\"") })
    );
    assert!(!batch.idempotency_key().is_empty());
}

#[test]
fn seed_batch_rejects_foreign_types_duplicate_names_and_uids() {
    let foreign = GuestLocalSeedResource::new(
        ResourceRef::parse("Zone/work").unwrap(),
        guest_ref(),
        payload("work", "Zone", ""),
    );
    assert_eq!(
        foreign.unwrap_err(),
        GuestLocalSeedResourceError::TypeNotApproved
    );

    let duplicate = GuestLocalSeedBatch::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        "seed-operation",
        vec![
            resource("gateway-agent", "Process", ""),
            resource("gateway-agent", "Process", ""),
        ],
    );
    assert_eq!(duplicate.unwrap_err(), GuestLocalError::SeedInvalid);

    let mut with_uid: serde_json::Value =
        serde_json::from_slice(&payload("gateway-agent", "Process", "")).unwrap();
    with_uid["metadata"]["uid"] =
        serde_json::Value::String("323e4567-e89b-42d3-a456-426614174002".to_owned());
    let with_uid = CanonicalJsonValue::parse(&serde_json::to_vec(&with_uid).unwrap())
        .unwrap()
        .to_canonical_bytes();
    assert_eq!(
        GuestLocalSeedResource::new(
            ResourceRef::parse("Process/gateway-agent").unwrap(),
            guest_ref(),
            with_uid,
        )
        .unwrap_err(),
        GuestLocalSeedResourceError::UidNotAllowed
    );
}

#[test]
fn seed_batch_rejects_private_effect_payloads_and_bad_relationships() {
    let private = GuestLocalSeedResource::new(
        ResourceRef::parse("Process/gateway-agent").unwrap(),
        guest_ref(),
        payload(
            "gateway-agent",
            "Process",
            r#""socketPath":"/run/private.sock""#,
        ),
    );
    assert_eq!(
        private.unwrap_err(),
        GuestLocalSeedResourceError::PrivateField
    );

    let wrong_execution = GuestLocalSeedResource::new(
        ResourceRef::parse("Process/gateway-agent").unwrap(),
        guest_ref(),
        payload(
            "gateway-agent",
            "Process",
            r#""executionRef":"Guest/other""#,
        ),
    );
    assert_eq!(
        wrong_execution.unwrap_err(),
        GuestLocalSeedResourceError::RelationshipMismatch
    );

    let wrong_owner = GuestLocalSeedResource::new(
        ResourceRef::parse("Process/gateway-agent").unwrap(),
        ResourceRef::parse("Guest/other").unwrap(),
        payload("gateway-agent", "Process", ""),
    );
    assert_eq!(
        wrong_owner.unwrap_err(),
        GuestLocalSeedResourceError::OwnerMismatch
    );

    let invalid_operation = GuestLocalSeedBatch::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        "seed operation",
        vec![resource("gateway-agent", "Process", "")],
    );
    assert_eq!(invalid_operation.unwrap_err(), GuestLocalError::SeedInvalid);
}
