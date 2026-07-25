//! Canonical d2b v3 resource-plane contracts.

pub mod identity;
pub mod resource_ref;

pub use identity::{
    AuthenticatedSubjectContext, BindingDigest, ControllerGeneration, EvidenceClass, IdentityError,
    Locality, ObservedGeneration, ReconnectGeneration, ResourceGeneration, ResourceName,
    ResourceTypeName, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
    Timestamp, TranscriptHash, TransportBinding, ZoneId, ZoneRevision,
};
pub use resource_ref::{ResourceRef, ResourceRefError};
