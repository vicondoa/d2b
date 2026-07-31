//! Canonical d2b v3 resource-plane contracts.

pub mod component_session;
pub mod credential;
pub mod credential_controller;
pub mod device;
pub mod error;
pub mod execution_policy;
pub mod guest;
pub mod host;
pub mod identity;
pub mod ifname;
pub mod limits;
pub mod network;
pub mod process;
pub mod provider;
pub mod resource;
pub mod resource_ref;
pub mod resource_schema;
pub mod resource_status;
pub mod semantic_services;
pub mod user;
pub mod volume;
pub mod volume_state;
pub mod zone_routing;
pub mod zone_session;

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

// The `provider` module's re-exports. Keep every `pub use provider::...` line
// inside this region so it stays one contiguous block.
pub use provider::{
    ArtifactDigest, ArtifactDigestSet, ArtifactId, BindingTargetType, CapabilitySupport,
    CompatibilityRange, ComponentDescriptor, ComponentType, DependencyAlias, DependencyDeclaration,
    Exportability, ExtensionSchemaRegistration, MAX_ARTIFACT_ID_BYTES,
    MAX_CAPABILITY_MATRIX_ENTRIES, MAX_COMPONENT_METHODS, MAX_COMPONENT_RESOURCE_TYPES,
    MAX_PROJECTION_FACTORIES, MAX_PROJECTION_REF_TYPES, MAX_PROVIDER_API_BINDINGS,
    MAX_PROVIDER_COMPONENTS, MAX_PUBLISHER_ID_BYTES, PROVIDER_RESOURCE_TYPE, PolicyEvaluation,
    ProjectionFactory, ProviderContractError, ProviderManifest, ProviderSpec, ResourceApiBinding,
    RevocationState, SignatureState, SpecifiedProviderMethod, StandardCapabilityMatrix,
    TrustEvidence, UNSUPPORTED_CAPABILITY_CODE, UpgradeDisposition,
    UpgradePolicy as ProviderUpgradePolicy,
};

// The `semantic_services` module's re-exports. Keep every
// `pub use semantic_services::...` line inside this region so it stays one
// contiguous block.
pub use semantic_services::{
    PROVIDER_REF_FIELD, SEMANTIC_BASE_SCHEMA_MAJOR, SEMANTIC_BASE_SCHEMA_MINOR,
    SEMANTIC_PROJECTION_PROTOCOL_VERSION, SemanticContractError, SemanticFamily, SemanticLayer,
    SemanticLayerSchema, SemanticPairContract, SemanticProjectionBinding, SemanticRole,
    SemanticSchemaId, SemanticTypeContract, UPDATE_POLICY_FIELD,
    catalog as semantic_service_catalog,
};

// The `volume_state` module's re-exports. Keep every
// `pub use volume_state::...` line inside this region so it stays one
// contiguous block.
// (empty until `ADR046-pstate-001` lands)

// The `ifname` module's re-exports. Keep every `pub use ifname::...` line
// inside this region so it stays one contiguous block.
// (empty until `ADR046-network-001` lands)

// The `credential_controller` module's re-exports. Keep every
// `pub use credential_controller::...` line inside this region so it stays one
// contiguous block.
// (empty until `ADR046-credential-006` lands)
