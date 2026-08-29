//! Strict private setup descriptor for Cloud Hypervisor Guests.
//!
//! The public Guest resource selects a system artifact and Provider settings.
//! This module carries the provider-private, signed semantic commitment that
//! later reconciliation uses. It deliberately has no host paths, locators,
//! credentials, executable arguments, numeric identities, or broker actions.

use std::fmt;

use d2b_contracts_provider::v3::ArtifactDigest;
use d2b_contracts_resource::v3::{
    ArtifactId, CanonicalJsonError, CanonicalJsonValue, ResourceGeneration, ResourceRef,
    SchemaFingerprint, SchemaVersion,
    execution_policy::BoundedToken,
    resource_schema::{canonical_json_bytes, framed_canonical_digest},
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::identity::ChildRoleSet;

/// Domain tag for private Cloud Hypervisor setup descriptor digests.
pub const GUEST_SETUP_DESCRIPTOR_DOMAIN_TAG: &str = "d2b:v3:ch-guest-setup-descriptor";
/// The only descriptor schema version currently admitted.
pub const GUEST_SETUP_DESCRIPTOR_SCHEMA_VERSION: (u32, u32) = (1, 0);
/// The only admitted descriptor signature algorithm.
pub const GUEST_SETUP_DESCRIPTOR_SIGNATURE_ALGORITHM: &str = "ed25519-blake3";
/// Maximum bytes in an opaque descriptor signature.
pub const MAX_DESCRIPTOR_SIGNATURE_BYTES: usize = 4096;
/// Maximum bootstrap handoff lifetime in milliseconds.
pub const MAX_BOOTSTRAP_HANDOFF_EXPIRY_MS: u64 = 86_400_000;

const CLOUD_HYPERVISOR_PROVIDER_REF: &str = "Provider/runtime-cloud-hypervisor";
const GUEST_RESOURCE_SEED_SCHEMA: &str = "guest-resource-seed";
const OPAQUE_BOOTSTRAP_HANDOFF_CLASS: &str = "opaque-bootstrap";

/// Closed failures while loading or validating a private setup descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSetupDescriptorError {
    /// The descriptor bytes were not canonical JSON.
    NonCanonical,
    /// The descriptor did not decode against its closed wire schema.
    InvalidEncoding,
    /// A field failed its bounded semantic validation.
    InvalidField,
    /// The descriptor selected a different Provider.
    ProviderMismatch,
    /// The descriptor schema version is not supported.
    SchemaVersionMismatch,
    /// The fixed direct-child role set was changed.
    ChildRolesMismatch,
    /// The descriptor digest did not match its signed semantic payload.
    DigestMismatch,
    /// The signature envelope was absent or invalid.
    SignatureInvalid,
    /// Canonical JSON encoding failed after validation.
    CanonicalJson,
}

impl fmt::Display for GuestSetupDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonCanonical => "guest-setup-descriptor-non-canonical",
            Self::InvalidEncoding => "guest-setup-descriptor-invalid-encoding",
            Self::InvalidField => "guest-setup-descriptor-field-invalid",
            Self::ProviderMismatch => "guest-setup-descriptor-provider-mismatch",
            Self::SchemaVersionMismatch => "guest-setup-descriptor-schema-version-mismatch",
            Self::ChildRolesMismatch => "guest-setup-descriptor-child-roles-mismatch",
            Self::DigestMismatch => "guest-setup-descriptor-digest-mismatch",
            Self::SignatureInvalid => "guest-setup-descriptor-signature-invalid",
            Self::CanonicalJson => "guest-setup-descriptor-canonical-json",
        })
    }
}

impl std::error::Error for GuestSetupDescriptorError {}

impl From<CanonicalJsonError> for GuestSetupDescriptorError {
    fn from(_: CanonicalJsonError) -> Self {
        Self::CanonicalJson
    }
}

/// The one signature algorithm accepted for a private setup descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureAlgorithm {
    /// Ed25519 over the descriptor digest with the d2b transcript framing.
    Ed25519Blake3,
}

impl SignatureAlgorithm {
    /// Return the canonical wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ed25519Blake3 => GUEST_SETUP_DESCRIPTOR_SIGNATURE_ALGORITHM,
        }
    }
}

/// The opaque signature value carried by the private descriptor.
///
/// The value is never rendered through `Debug` and is not interpreted by the
/// Cloud Hypervisor controller. Trust and cryptographic verification remain
/// owned by the signed catalog loader.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OpaqueDescriptorSignature(String);

impl OpaqueDescriptorSignature {
    /// Validate one opaque printable signature value.
    pub fn parse(value: impl Into<String>) -> Result<Self, GuestSetupDescriptorError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_DESCRIPTOR_SIGNATURE_BYTES
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(GuestSetupDescriptorError::SignatureInvalid);
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for OpaqueDescriptorSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueDescriptorSignature(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for OpaqueDescriptorSignature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// The signed signature envelope for a setup descriptor.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescriptorSignature {
    algorithm: SignatureAlgorithm,
    key_fingerprint: SchemaFingerprint,
    signature: OpaqueDescriptorSignature,
}

impl DescriptorSignature {
    /// Construct a signature envelope from its catalog-bound fields.
    pub fn new(
        algorithm: SignatureAlgorithm,
        key_fingerprint: SchemaFingerprint,
        signature: impl Into<String>,
    ) -> Result<Self, GuestSetupDescriptorError> {
        if algorithm.as_str() != GUEST_SETUP_DESCRIPTOR_SIGNATURE_ALGORITHM {
            return Err(GuestSetupDescriptorError::SignatureInvalid);
        }
        Ok(Self {
            algorithm,
            key_fingerprint,
            signature: OpaqueDescriptorSignature::parse(signature)?,
        })
    }

    /// Return the signature algorithm.
    pub const fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }

    /// Borrow the signing-key fingerprint.
    pub const fn key_fingerprint(&self) -> &SchemaFingerprint {
        &self.key_fingerprint
    }
}

impl fmt::Debug for DescriptorSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DescriptorSignature(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for DescriptorSignature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            algorithm: SignatureAlgorithm,
            key_fingerprint: SchemaFingerprint,
            signature: OpaqueDescriptorSignature,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.algorithm, wire.key_fingerprint, wire.signature.0)
            .map_err(serde::de::Error::custom)
    }
}

/// The guest-local Resource API seed schema commitment.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestSeedContract {
    schema: BoundedToken,
    schema_version: SchemaVersion,
    fingerprint: SchemaFingerprint,
}

impl GuestSeedContract {
    /// Construct a semantic guest seed contract.
    pub fn new(
        schema: impl Into<String>,
        schema_version: SchemaVersion,
        fingerprint: SchemaFingerprint,
    ) -> Result<Self, GuestSetupDescriptorError> {
        let schema = BoundedToken::parse(schema.into())
            .map_err(|_| GuestSetupDescriptorError::InvalidField)?;
        if schema.as_str() != GUEST_RESOURCE_SEED_SCHEMA {
            return Err(GuestSetupDescriptorError::InvalidField);
        }
        Ok(Self {
            schema,
            schema_version,
            fingerprint,
        })
    }

    /// Borrow the seed schema identifier.
    pub const fn schema(&self) -> &BoundedToken {
        &self.schema
    }

    /// Return the seed schema version.
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Borrow the signed seed schema fingerprint.
    pub const fn fingerprint(&self) -> &SchemaFingerprint {
        &self.fingerprint
    }
}

impl fmt::Debug for GuestSeedContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestSeedContract(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for GuestSeedContract {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema: BoundedToken,
            schema_version: SchemaVersion,
            fingerprint: SchemaFingerprint,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.schema.as_str(), wire.schema_version, wire.fingerprint)
            .map_err(serde::de::Error::custom)
    }
}

/// An opaque, time-bounded bootstrap handoff class.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapHandoff {
    class: BoundedToken,
    expiry_ms: u64,
}

impl BootstrapHandoff {
    /// Construct a bounded semantic bootstrap handoff policy.
    pub fn new(
        class: impl Into<String>,
        expiry_ms: u64,
    ) -> Result<Self, GuestSetupDescriptorError> {
        if !(1..=MAX_BOOTSTRAP_HANDOFF_EXPIRY_MS).contains(&expiry_ms) {
            return Err(GuestSetupDescriptorError::InvalidField);
        }
        let class = BoundedToken::parse(class.into())
            .map_err(|_| GuestSetupDescriptorError::InvalidField)?;
        if class.as_str() != OPAQUE_BOOTSTRAP_HANDOFF_CLASS {
            return Err(GuestSetupDescriptorError::InvalidField);
        }
        Ok(Self { class, expiry_ms })
    }

    /// Borrow the opaque handoff class.
    pub const fn class(&self) -> &BoundedToken {
        &self.class
    }

    /// Return the maximum handoff lifetime in milliseconds.
    pub const fn expiry_ms(&self) -> u64 {
        self.expiry_ms
    }
}

impl fmt::Debug for BootstrapHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapHandoff(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for BootstrapHandoff {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            class: BoundedToken,
            expiry_ms: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.class.as_str(), wire.expiry_ms).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedDescriptor<'a> {
    schema_version: SchemaVersion,
    signature_algorithm: SignatureAlgorithm,
    signature_key_fingerprint: &'a SchemaFingerprint,
    provider_ref: &'a ResourceRef,
    provider_generation: ResourceGeneration,
    system_artifact_id: &'a ArtifactId,
    system_artifact_commitment: &'a ArtifactDigest,
    child_roles: &'a ChildRoleSet,
    seed: &'a GuestSeedContract,
    bootstrap_handoff: &'a BootstrapHandoff,
}

/// A signed, immutable, provider-private Guest setup descriptor.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestSetupDescriptor {
    schema_version: SchemaVersion,
    descriptor_digest: SchemaFingerprint,
    provider_ref: ResourceRef,
    provider_generation: ResourceGeneration,
    system_artifact_id: ArtifactId,
    system_artifact_commitment: ArtifactDigest,
    child_roles: ChildRoleSet,
    seed: GuestSeedContract,
    bootstrap_handoff: BootstrapHandoff,
    signature: DescriptorSignature,
}

/// Cryptographic trust boundary for private setup descriptors.
pub trait GuestSetupDescriptorVerifier {
    /// Verify the descriptor signature against the catalog-owned trust root.
    fn verify(
        &self,
        key_fingerprint: &SchemaFingerprint,
        descriptor_digest: &SchemaFingerprint,
        signature: &str,
    ) -> bool;
}

/// A setup descriptor whose signature was accepted by a trusted verifier.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedGuestSetupDescriptor(GuestSetupDescriptor);

impl VerifiedGuestSetupDescriptor {
    /// Borrow the verified descriptor.
    pub const fn descriptor(&self) -> &GuestSetupDescriptor {
        &self.0
    }

    /// Render the verified descriptor envelope without exposing its signature
    /// through Debug.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GuestSetupDescriptorError> {
        self.0.canonical_bytes()
    }
}

impl fmt::Debug for VerifiedGuestSetupDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedGuestSetupDescriptor(<redacted>)")
    }
}

impl GuestSetupDescriptor {
    /// Construct a descriptor and compute its canonical semantic digest.
    pub fn new(
        provider_ref: ResourceRef,
        provider_generation: ResourceGeneration,
        system_artifact_id: ArtifactId,
        system_artifact_commitment: ArtifactDigest,
        seed: GuestSeedContract,
        bootstrap_handoff: BootstrapHandoff,
        signature: DescriptorSignature,
    ) -> Result<Self, GuestSetupDescriptorError> {
        let descriptor = Self {
            schema_version: SchemaVersion::new(
                GUEST_SETUP_DESCRIPTOR_SCHEMA_VERSION.0,
                GUEST_SETUP_DESCRIPTOR_SCHEMA_VERSION.1,
            )
            .map_err(|_| GuestSetupDescriptorError::SchemaVersionMismatch)?,
            descriptor_digest: SchemaFingerprint::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .map_err(|_| GuestSetupDescriptorError::CanonicalJson)?,
            provider_ref,
            provider_generation,
            system_artifact_id,
            system_artifact_commitment,
            child_roles: ChildRoleSet::fixed(),
            seed,
            bootstrap_handoff,
            signature,
        };
        descriptor.validate_fields()?;
        let digest = descriptor.computed_digest()?;
        let descriptor = Self {
            descriptor_digest: SchemaFingerprint::parse(digest)
                .map_err(|_| GuestSetupDescriptorError::CanonicalJson)?,
            ..descriptor
        };
        descriptor.validate_integrity()?;
        Ok(descriptor)
    }

    /// Parse exactly canonical descriptor bytes.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, GuestSetupDescriptorError> {
        let value = CanonicalJsonValue::parse(bytes)?;
        if value.to_canonical_bytes() != bytes {
            return Err(GuestSetupDescriptorError::NonCanonical);
        }
        serde_json::from_slice(bytes).map_err(|_| GuestSetupDescriptorError::InvalidEncoding)
    }

    /// Render the exact canonical descriptor envelope bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GuestSetupDescriptorError> {
        self.validate_integrity()?;
        Ok(CanonicalJsonValue::parse(
            &serde_json::to_vec(self).map_err(|_| GuestSetupDescriptorError::CanonicalJson)?,
        )?
        .to_canonical_bytes())
    }

    /// Validate the closed fields and self-consistent semantic digest.
    pub fn validate_integrity(&self) -> Result<(), GuestSetupDescriptorError> {
        self.validate_fields()?;
        if self.computed_digest()? != self.descriptor_digest.as_str() {
            return Err(GuestSetupDescriptorError::DigestMismatch);
        }
        Ok(())
    }

    /// Verify the catalog-bound signature and return the only child-planning
    /// descriptor type.
    pub fn verify_with(
        &self,
        verifier: &impl GuestSetupDescriptorVerifier,
    ) -> Result<VerifiedGuestSetupDescriptor, GuestSetupDescriptorError> {
        self.validate_integrity()?;
        if !verifier.verify(
            &self.signature.key_fingerprint,
            &self.descriptor_digest,
            &self.signature.signature.0,
        ) {
            return Err(GuestSetupDescriptorError::SignatureInvalid);
        }
        Ok(VerifiedGuestSetupDescriptor(self.clone()))
    }

    /// Borrow the selected Provider identity.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Return the selected Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Borrow the selected public system artifact ID.
    pub const fn system_artifact_id(&self) -> &ArtifactId {
        &self.system_artifact_id
    }

    /// Borrow the private system artifact commitment.
    pub const fn system_artifact_commitment(&self) -> &ArtifactDigest {
        &self.system_artifact_commitment
    }

    /// Borrow the fixed direct-child role set.
    pub const fn child_roles(&self) -> &ChildRoleSet {
        &self.child_roles
    }

    /// Borrow the guest-local seed contract.
    pub const fn seed(&self) -> &GuestSeedContract {
        &self.seed
    }

    /// Borrow the opaque bootstrap handoff policy.
    pub const fn bootstrap_handoff(&self) -> &BootstrapHandoff {
        &self.bootstrap_handoff
    }

    /// Borrow the canonical descriptor digest.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    fn validate_fields(&self) -> Result<(), GuestSetupDescriptorError> {
        let expected_provider = ResourceRef::parse(CLOUD_HYPERVISOR_PROVIDER_REF)
            .map_err(|_| GuestSetupDescriptorError::ProviderMismatch)?;
        if self.provider_ref != expected_provider {
            return Err(GuestSetupDescriptorError::ProviderMismatch);
        }
        let expected_version = SchemaVersion::new(
            GUEST_SETUP_DESCRIPTOR_SCHEMA_VERSION.0,
            GUEST_SETUP_DESCRIPTOR_SCHEMA_VERSION.1,
        )
        .map_err(|_| GuestSetupDescriptorError::SchemaVersionMismatch)?;
        if self.schema_version != expected_version {
            return Err(GuestSetupDescriptorError::SchemaVersionMismatch);
        }
        if !self.child_roles.is_fixed() {
            return Err(GuestSetupDescriptorError::ChildRolesMismatch);
        }
        if self.signature.algorithm != SignatureAlgorithm::Ed25519Blake3 {
            return Err(GuestSetupDescriptorError::SignatureInvalid);
        }
        Ok(())
    }

    fn computed_digest(&self) -> Result<String, GuestSetupDescriptorError> {
        let unsigned = UnsignedDescriptor {
            schema_version: self.schema_version,
            signature_algorithm: self.signature.algorithm,
            signature_key_fingerprint: &self.signature.key_fingerprint,
            provider_ref: &self.provider_ref,
            provider_generation: self.provider_generation,
            system_artifact_id: &self.system_artifact_id,
            system_artifact_commitment: &self.system_artifact_commitment,
            child_roles: &self.child_roles,
            seed: &self.seed,
            bootstrap_handoff: &self.bootstrap_handoff,
        };
        let bytes = canonical_json_bytes(&unsigned)?;
        Ok(framed_canonical_digest(
            GUEST_SETUP_DESCRIPTOR_DOMAIN_TAG,
            &bytes,
        ))
    }
}

impl fmt::Debug for GuestSetupDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestSetupDescriptor(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for GuestSetupDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema_version: SchemaVersion,
            descriptor_digest: SchemaFingerprint,
            provider_ref: ResourceRef,
            provider_generation: ResourceGeneration,
            system_artifact_id: ArtifactId,
            system_artifact_commitment: ArtifactDigest,
            child_roles: ChildRoleSet,
            seed: GuestSeedContract,
            bootstrap_handoff: BootstrapHandoff,
            signature: DescriptorSignature,
        }

        let wire = Wire::deserialize(deserializer)?;
        let descriptor = Self {
            schema_version: wire.schema_version,
            descriptor_digest: wire.descriptor_digest,
            provider_ref: wire.provider_ref,
            provider_generation: wire.provider_generation,
            system_artifact_id: wire.system_artifact_id,
            system_artifact_commitment: wire.system_artifact_commitment,
            child_roles: wire.child_roles,
            seed: wire.seed,
            bootstrap_handoff: wire.bootstrap_handoff,
            signature: wire.signature,
        };
        descriptor
            .validate_integrity()
            .map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}
