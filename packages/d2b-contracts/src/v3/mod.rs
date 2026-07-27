//! Canonical d2b v3 resource-plane contracts.

pub mod error;
pub mod identity;
pub mod limits;
pub mod resource;
pub mod resource_ref;
pub mod resource_schema;
pub mod resource_status;

pub use error::{
    MAX_RESOURCE_ERROR_REASON_BYTES, MAX_RESOURCE_ERROR_RETRY_AFTER_MS, ResourceError,
    ResourceErrorKind, ResourceErrorReason, ResourceErrorValidation, RetryClass,
};
pub use identity::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
    EvidenceClass, IdentityError, Locality, ObservedGeneration,
    RESOURCE_BUNDLE_GENERATION_DOMAIN_TAG, ReconnectGeneration, ResourceBundleGenerationId,
    ResourceGeneration, ResourceName, ResourceTypeName, ResourceUid, SchemaFingerprint,
    ServiceName, SessionBinding, SessionPurpose, Timestamp, TranscriptHash, TransportBinding,
    ZoneId, ZoneRevision,
};
pub use limits::*;
pub use resource::{
    DisruptiveUpdateMode, FinalizerId, ManagedBy, NonDisruptiveUpdateMode, PresentationMetadata,
    ProviderSpecExtension, ResourceEnvelope, ResourceError as ResourceObjectError,
    ResourceMetadata, ResourceSpec, UpdatePolicy,
};
pub use resource_ref::{ResourceRef, ResourceRefError};
pub use resource_schema::{
    BaseSchemaBinding, BaseSchemaIdentity, CANONICAL_JSON_PROFILE, CanonicalJsonCodecReason,
    CanonicalJsonError, CanonicalJsonObject, CanonicalJsonValue, ExtensionSchemaId,
    ExtensionSchemaLayer, ObjectFieldSchema, ProviderExtensionRegistration,
    RESOURCE_ENVELOPE_DOMAIN_TAG, RESOURCE_SPEC_DOMAIN_TAG, RESOURCE_STATUS_DOMAIN_TAG,
    ResourceSchemaContract, ResourceSchemaError, SCHEMA_DOMAIN_TAG, SchemaVersion,
    canonical_digest, canonical_json_bytes,
};
pub use resource_status::{
    ConditionState, ProviderStatusExtension, ResourceCondition, ResourceCurrencySet,
    ResourceOutcome, ResourcePhase, ResourceStatus, ResourceStatusError, ResourceUpdateStatus,
    StatusCode, StatusMessage, UpdateDisruption, UpdateReason, UpdateState,
};
