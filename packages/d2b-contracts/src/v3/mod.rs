//! Canonical d2b v3 resource-plane contracts.

pub mod identity;
pub mod resource_ref;

pub use identity::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
    EvidenceClass, IdentityError, Locality, ObservedGeneration,
    RESOURCE_BUNDLE_GENERATION_DOMAIN_TAG, ReconnectGeneration, ResourceBundleGenerationId,
    ResourceGeneration, ResourceName, ResourceTypeName, ResourceUid, SchemaFingerprint,
    ServiceName, SessionBinding, SessionPurpose, Timestamp, TranscriptHash, TransportBinding,
    ZoneId, ZoneRevision,
};
pub use resource_ref::{ResourceRef, ResourceRefError};
