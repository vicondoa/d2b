use d2b_contracts_provider::v3::ArtifactDigest;
use d2b_contracts_resource::v3::{
    ArtifactId, ResourceGeneration, ResourceRef, SchemaFingerprint, SchemaVersion, ZoneId,
};
use d2b_provider_runtime_cloud_hypervisor::identity::derive_private_runtime_scope;
use d2b_provider_runtime_cloud_hypervisor::{
    BootstrapHandoff, DescriptorSignature, GuestChildBatch, GuestSeedContract,
    GuestSetupDescriptor, GuestSetupDescriptorVerifier, SignatureAlgorithm,
};

const DIGEST: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct TestVerifier;

impl GuestSetupDescriptorVerifier for TestVerifier {
    fn verify(
        &self,
        _key_fingerprint: &SchemaFingerprint,
        _descriptor_digest: &SchemaFingerprint,
        signature: &str,
    ) -> bool {
        signature == "private-signature-sentinel" || signature == "signature-sentinel"
    }
}

#[test]
fn private_descriptor_and_runtime_scope_debug_are_redacted() {
    let descriptor = GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(2).unwrap(),
        ArtifactId::parse("guest-system").unwrap(),
        ArtifactDigest::parse(DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(DIGEST).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 10_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            SchemaFingerprint::parse(DIGEST).unwrap(),
            "private-signature-sentinel",
        )
        .unwrap(),
    )
    .unwrap();
    let rendered = format!("{descriptor:?}");
    assert!(!rendered.contains("opaque-bootstrap"));
    assert!(!rendered.contains("private-signature-sentinel"));

    let zone_uid =
        d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let guest_uid =
        d2b_contracts_resource::v3::ResourceUid::parse("323e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let scope = derive_private_runtime_scope(
        &zone_uid,
        &guest_uid,
        "vmm",
        ResourceGeneration::new(2).unwrap(),
    )
    .unwrap();
    let rendered = format!("{scope:?}");
    assert!(!rendered.contains("223e4567"));
    assert!(!rendered.contains("323e4567"));
    assert!(!rendered.contains("vmm"));
}

#[test]
fn runtime_scope_changes_for_zone_and_guest_reincarnation() {
    let zone_a =
        d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let zone_b =
        d2b_contracts_resource::v3::ResourceUid::parse("423e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let guest_a =
        d2b_contracts_resource::v3::ResourceUid::parse("323e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let guest_b =
        d2b_contracts_resource::v3::ResourceUid::parse("523e4567-e89b-42d3-a456-426614174000")
            .unwrap();
    let generation = ResourceGeneration::new(1).unwrap();
    let first = derive_private_runtime_scope(&zone_a, &guest_a, "vmm", generation).unwrap();
    assert_ne!(
        first,
        derive_private_runtime_scope(&zone_b, &guest_a, "vmm", generation).unwrap()
    );
    assert_ne!(
        first,
        derive_private_runtime_scope(&zone_a, &guest_b, "vmm", generation).unwrap()
    );
    assert_ne!(
        first,
        derive_private_runtime_scope(
            &zone_a,
            &guest_a,
            "vmm",
            ResourceGeneration::new(2).unwrap()
        )
        .unwrap()
    );
}

#[test]
fn child_batch_debug_and_wire_bytes_do_not_include_descriptor_payload() {
    let descriptor = GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(2).unwrap(),
        ArtifactId::parse("guest-system").unwrap(),
        ArtifactDigest::parse(DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(DIGEST).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 10_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            SchemaFingerprint::parse(DIGEST).unwrap(),
            "signature-sentinel",
        )
        .unwrap(),
    )
    .unwrap();
    let descriptor = descriptor.verify_with(&TestVerifier).unwrap();
    let batch = GuestChildBatch::from_descriptor(
        ZoneId::parse("dev").unwrap(),
        ResourceRef::parse("Guest/gateway").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        &descriptor,
    )
    .unwrap();
    let rendered = format!("{batch:?}");
    let bytes = String::from_utf8(batch.canonical_bytes().unwrap()).unwrap();
    for output in [rendered, bytes] {
        assert!(!output.contains("opaque-bootstrap"));
        assert!(!output.contains("signature-sentinel"));
    }
}
