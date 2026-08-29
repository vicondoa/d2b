use d2b_contracts_provider::v3::ArtifactDigest;
use d2b_contracts_resource::v3::{
    ArtifactId, ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, SchemaVersion,
    ZoneId, ZoneRevision,
};
use d2b_provider_runtime_cloud_hypervisor::{
    BootstrapHandoff, ChildRole, DescriptorSignature, GuestChildBatch, GuestSeedContract,
    GuestSetupDescriptor, GuestSetupDescriptorVerifier, SignatureAlgorithm, map_commit_response,
};

const ARTIFACT_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SCHEMA_FINGERPRINT: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn descriptor() -> GuestSetupDescriptor {
    GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(3).unwrap(),
        ArtifactId::parse("guest-system").unwrap(),
        ArtifactDigest::parse(ARTIFACT_DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
            "signature-sentinel",
        )
        .unwrap(),
    )
    .unwrap()
}

struct TestVerifier;

impl GuestSetupDescriptorVerifier for TestVerifier {
    fn verify(
        &self,
        _key_fingerprint: &SchemaFingerprint,
        _descriptor_digest: &SchemaFingerprint,
        signature: &str,
    ) -> bool {
        signature == "signature-sentinel"
    }
}

#[test]
fn descriptor_is_canonical_and_requires_signature_verification() {
    let descriptor = descriptor();
    let canonical = descriptor.canonical_bytes().unwrap();
    assert_eq!(
        GuestSetupDescriptor::from_canonical_bytes(&canonical).unwrap(),
        descriptor
    );
    assert!(descriptor.verify_with(&TestVerifier).is_ok());
    assert!(
        String::from_utf8(canonical)
            .unwrap()
            .contains("descriptorDigest")
    );
    let noncanonical = serde_json::to_vec(&serde_json::json!({
        "signature": {
            "algorithm": "ed25519-blake3",
            "keyFingerprint": SCHEMA_FINGERPRINT,
            "signature": "signature-sentinel"
        },
        "schemaVersion": "1.0",
        "descriptorDigest": descriptor.descriptor_digest().as_str(),
        "providerRef": "Provider/runtime-cloud-hypervisor",
        "providerGeneration": 3,
        "systemArtifactId": "guest-system",
        "systemArtifactCommitment": ARTIFACT_DIGEST,
        "childRoles": ["vmm", "ch-api", "guest-control", "system"],
        "seed": {
            "schema": "guest-resource-seed",
            "schemaVersion": "1.0",
            "fingerprint": SCHEMA_FINGERPRINT
        },
        "bootstrapHandoff": {
            "class": "opaque-bootstrap",
            "expiryMs": 30_000
        }
    }))
    .unwrap();
    let mut noncanonical_with_whitespace = vec![b' '];
    noncanonical_with_whitespace.extend(noncanonical);
    assert!(GuestSetupDescriptor::from_canonical_bytes(&noncanonical_with_whitespace).is_err());

    let mut tampered: serde_json::Value =
        serde_json::from_slice(&descriptor.canonical_bytes().unwrap()).unwrap();
    tampered.as_object_mut().unwrap().insert(
        "systemArtifactId".to_owned(),
        serde_json::Value::String("other-system".to_owned()),
    );
    assert!(
        GuestSetupDescriptor::from_canonical_bytes(&serde_json::to_vec(&tampered).unwrap())
            .is_err()
    );
}

#[test]
fn child_planning_rejects_unverified_and_forged_descriptors() {
    let forged = GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(3).unwrap(),
        ArtifactId::parse("other-system").unwrap(),
        ArtifactDigest::parse(ARTIFACT_DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
            "forged-signature",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(forged.verify_with(&TestVerifier).is_err());
}

#[test]
fn descriptor_semantic_tokens_are_exact() {
    assert!(
        GuestSeedContract::new(
            "support",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
        )
        .is_err()
    );
    assert!(BootstrapHandoff::new("allocator", 30_000).is_err());
    assert!(
        d2b_provider_runtime_cloud_hypervisor::identity::derive_private_runtime_scope(
            &ResourceUid::parse("223e4567-e89b-42d3-a456-426614174000").unwrap(),
            &ResourceUid::parse("323e4567-e89b-42d3-a456-426614174000").unwrap(),
            "socket",
            ResourceGeneration::new(1).unwrap(),
        )
        .is_err()
    );
}

#[test]
fn descriptor_rejects_private_effect_inputs_and_unknown_fields() {
    let descriptor = descriptor().verify_with(&TestVerifier).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&descriptor.canonical_bytes().unwrap()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.insert(
        "argv".to_owned(),
        serde_json::Value::Array(vec![serde_json::Value::String("--bad".to_owned())]),
    );
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(GuestSetupDescriptor::from_canonical_bytes(&bytes).is_err());

    let mut value: serde_json::Value =
        serde_json::from_slice(&descriptor.canonical_bytes().unwrap()).unwrap();
    value.as_object_mut().unwrap().insert(
        "storePath".to_owned(),
        serde_json::Value::String("/nix/store/x".to_owned()),
    );
    assert!(
        GuestSetupDescriptor::from_canonical_bytes(&serde_json::to_vec(&value).unwrap()).is_err()
    );

    let mut value: serde_json::Value =
        serde_json::from_slice(&descriptor.canonical_bytes().unwrap()).unwrap();
    value.as_object_mut().unwrap().insert(
        "providerGeneration".to_owned(),
        serde_json::Value::Number(0.into()),
    );
    assert!(
        GuestSetupDescriptor::from_canonical_bytes(&serde_json::to_vec(&value).unwrap()).is_err()
    );
}

#[test]
fn fixed_guest_child_batch_is_name_addressed_and_uid_free() {
    let descriptor = descriptor().verify_with(&TestVerifier).unwrap();
    let zone = ZoneId::parse("dev").unwrap();
    let guest = ResourceRef::parse("Guest/gateway").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let batch =
        GuestChildBatch::from_descriptor(zone.clone(), guest.clone(), execution, &descriptor)
            .unwrap();

    assert_eq!(
        batch.child_ref(ChildRole::VmmProcess).unwrap(),
        &ResourceRef::parse("Process/gateway-vmm").unwrap()
    );
    assert_eq!(
        batch.child_ref(ChildRole::ChApiEndpoint).unwrap(),
        &ResourceRef::parse("Endpoint/gateway-ch-api").unwrap()
    );
    assert_eq!(
        batch.child_ref(ChildRole::GuestControlEndpoint).unwrap(),
        &ResourceRef::parse("Endpoint/gateway-guest-control").unwrap()
    );
    assert_eq!(
        batch.child_ref(ChildRole::SystemVolume).unwrap(),
        &ResourceRef::parse("Volume/gateway-system").unwrap()
    );
    assert_eq!(batch.mutations().len(), 4);
    assert!(batch.mutations().iter().all(|mutation| {
        mutation.precondition()
            == d2b_provider_runtime_cloud_hypervisor::CreatePrecondition::CreateAbsent
            && mutation.expected_uid().is_none()
            && mutation.owner_ref() == &guest
            && mutation.zone() == &zone
    }));
    assert!(
        batch
            .canonical_bytes()
            .unwrap()
            .windows(3)
            .all(|window| window != b"uid")
    );
}

#[test]
fn commit_response_maps_every_child_and_fences_bad_rows() {
    let descriptor = descriptor().verify_with(&TestVerifier).unwrap();
    let zone = ZoneId::parse("dev").unwrap();
    let guest = ResourceRef::parse("Guest/gateway").unwrap();
    let batch = GuestChildBatch::from_descriptor(
        zone.clone(),
        guest.clone(),
        ResourceRef::parse("Host/host-system").unwrap(),
        &descriptor,
    )
    .unwrap();
    let returned = batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                mutation.target().clone(),
                mutation.owner_ref().clone(),
                zone.clone(),
                ResourceUid::parse(format!("123e4567-e89b-42d3-a456-4266141740{index:02}"))
                    .unwrap(),
                ZoneRevision::new(9),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let committed = map_commit_response(&batch, returned.clone()).unwrap();
    assert_eq!(committed.len(), batch.mutations().len());
    assert_eq!(
        committed
            .get(batch.child_ref(ChildRole::VmmProcess).unwrap())
            .unwrap()
            .revision(),
        ZoneRevision::new(9)
    );

    let mut missing = returned.clone();
    missing.pop();
    assert!(map_commit_response(&batch, missing).is_err());

    let mut duplicate = returned;
    duplicate.push(duplicate[0].clone());
    assert!(map_commit_response(&batch, duplicate).is_err());

    let wrong_owner = batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                mutation.target().clone(),
                ResourceRef::parse("Guest/other").unwrap(),
                zone.clone(),
                ResourceUid::parse(format!("523e4567-e89b-42d3-a456-4266141740{index:02}"))
                    .unwrap(),
                ZoneRevision::new(9),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        map_commit_response(&batch, wrong_owner),
        Err(d2b_provider_runtime_cloud_hypervisor::CommitResponseError::WrongOwner)
    );

    let wrong_zone = batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                mutation.target().clone(),
                mutation.owner_ref().clone(),
                ZoneId::parse("other").unwrap(),
                ResourceUid::parse(format!("623e4567-e89b-42d3-a456-4266141740{index:02}"))
                    .unwrap(),
                ZoneRevision::new(9),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        map_commit_response(&batch, wrong_zone),
        Err(d2b_provider_runtime_cloud_hypervisor::CommitResponseError::WrongZone)
    );

    let mut wrong_type = batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                mutation.target().clone(),
                mutation.owner_ref().clone(),
                zone.clone(),
                ResourceUid::parse(format!("723e4567-e89b-42d3-a456-4266141740{index:02}"))
                    .unwrap(),
                ZoneRevision::new(9),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    wrong_type[0] = d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
        ResourceRef::parse("Endpoint/gateway-vmm").unwrap(),
        guest,
        zone,
        ResourceUid::parse("823e4567-e89b-42d3-a456-426614174000").unwrap(),
        ZoneRevision::new(9),
    )
    .unwrap();
    assert_eq!(
        map_commit_response(&batch, wrong_type),
        Err(d2b_provider_runtime_cloud_hypervisor::CommitResponseError::WrongType)
    );
}
