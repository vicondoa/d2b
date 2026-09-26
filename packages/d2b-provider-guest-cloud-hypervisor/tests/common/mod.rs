#![allow(dead_code)]

//! Harness shared by the Cloud Hypervisor controller test binaries.

use d2b_contracts_provider::v3::ArtifactDigest;
use d2b_contracts_resource::v3::{
    ArtifactId, ResourceGeneration, ResourceRef, SchemaFingerprint, SchemaVersion,
};
use d2b_provider_guest_cloud_hypervisor::{
    BootstrapHandoff, CloudHypervisorConfig, DescriptorSignature, GuestSeedContract,
    GuestSetupDescriptor, GuestSetupDescriptorVerifier, MachineType, SignatureAlgorithm,
    VerifiedGuestSetupDescriptor,
};

pub const ARTIFACT_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const SCHEMA_FINGERPRINT: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
pub const ZONE_UID: &str = "223e4567-e89b-42d3-a456-426614174001";

pub struct AcceptingVerifier;

impl GuestSetupDescriptorVerifier for AcceptingVerifier {
    fn verify(
        &self,
        _key_fingerprint: &SchemaFingerprint,
        _descriptor_digest: &SchemaFingerprint,
        signature: &str,
    ) -> bool {
        signature == "signature-sentinel"
    }
}

pub fn descriptor() -> VerifiedGuestSetupDescriptor {
    raw_descriptor("signature-sentinel")
        .verify_with(&AcceptingVerifier)
        .unwrap()
}

pub fn raw_descriptor(signature: &str) -> GuestSetupDescriptor {
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
            signature,
        )
        .unwrap(),
    )
    .unwrap()
}

pub fn config() -> CloudHypervisorConfig {
    CloudHypervisorConfig {
        controller_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        default_vcpus: 2,
        default_memory_mb: 512,
        default_machine_type: MachineType::Q35,
        watchdog: true,
        adoption_window_ms: 30_000,
        health_check_interval_ms: 30_000,
        health_check_timeout_ms: 5_000,
        health_check_failure_threshold: 3,
        startup_deadline_ms: 120_000,
    }
}
