//! Canonical d2b v3 resource-plane contracts.

pub mod component_session;
pub mod credential;
pub mod credential_controller;
pub mod device;
pub mod endpoint;
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
pub mod resource_bundle;
pub mod resource_ref;
pub mod resource_schema;
pub mod resource_status;
pub mod semantic_services;
pub mod storage;
pub mod user;
pub mod volume;
pub mod volume_state;
pub mod zone_routing;
pub mod zone_session;

pub use endpoint::{
    ENDPOINT_RESOURCE_TYPE, EndpointAttachmentPolicy, EndpointClass, EndpointConsumerPolicy,
    EndpointLifecyclePolicy, EndpointLocality, EndpointOperation, EndpointSpec, EndpointSpecError,
    EndpointTransport, EndpointVisibility, MAX_ENDPOINT_ATTACHMENTS, MAX_ENDPOINT_CONSUMER_ENTRIES,
    MAX_ENDPOINT_FINGERPRINT_BYTES, MAX_ENDPOINT_OPERATIONS, MAX_ENDPOINT_PROVIDER_COMPONENTS,
};
pub use error::{
    MAX_RESOURCE_ERROR_REASON_BYTES, MAX_RESOURCE_ERROR_RETRY_AFTER_MS, ResourceError,
    ResourceErrorKind, ResourceErrorReason, ResourceErrorValidation, RetryClass,
};
pub use guest::{GUEST_RESOURCE_TYPE, GuestSpec};
pub use host::{HOST_PROVIDER_REF, HOST_RESOURCE_TYPE, HostSpec, IsolationPosture};
pub use identity::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
    EvidenceClass, IdentityError, Locality, ObservedGeneration,
    RESOURCE_BUNDLE_GENERATION_DOMAIN_TAG, ReconnectGeneration, ResourceBundleGenerationId,
    ResourceGeneration, ResourceName, ResourceTypeName, ResourceUid, SchemaFingerprint,
    ServiceName, SessionBinding, SessionPurpose, Timestamp, TranscriptHash, TransportBinding,
    ZoneId, ZoneRevision,
};
pub use limits::*;
pub use process::{
    AdoptionPolicy, CapabilityClass, DesiredLifecycle, DeviceAccess, DeviceUsageSpec,
    EnvironmentClass, EphemeralProcessSpec, ExecutionSpec, HealthCheckClass, HealthCheckSpec,
    MappingClass, MountAccess, MountSpec, NamespaceClass, NetworkUsageSpec, PortProtocol, PortSpec,
    ProcessClass, ProcessSpec, ReadinessClass, ReadinessSpec, RestartClass, RestartPolicySpec,
    SandboxSpec, TelemetrySpec, UserNamespaceSpec,
};
pub use resource::{
    DisruptiveUpdateMode, FinalizerId, ManagedBy, NonDisruptiveUpdateMode, PresentationMetadata,
    ProviderSpecExtension, ResourceEnvelope, ResourceError as ResourceObjectError,
    ResourceMetadata, ResourceSpec, UpdatePolicy,
};
pub use resource_bundle::{
    ARTIFACT_CATALOG_DOMAIN_TAG, BundleIntegrityPin, BundleResource, BundleResourceMetadata,
    MAX_BUNDLE_FINGERPRINTS, MAX_BUNDLE_RESOURCES, RESOURCE_BUNDLE_CONTENT_DOMAIN_TAG,
    ResourceBundle, ResourceBundleError,
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
pub use user::{
    MAX_OS_GROUP_BYTES, MAX_OS_USERNAME_BYTES, MAX_USER_GROUPS, OsGroupName, OsUsername,
    USER_RESOURCE_TYPE, UserSpec,
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
pub use volume_state::{
    MAX_STATE_DOCUMENT_BYTES, MAX_STATE_GENERATION, MarkerStatus, MigrationPolicy,
    PersistenceClass, QuotaUsage, SealingStatus, SensitivityClass, StateDigest, StateEnvelope,
    StateSchemaPhase, VolumeStateError, VolumeStateSchema, VolumeStateSchemaId, VolumeStateStatus,
    canonical_state_payload_bytes, canonical_state_payload_digest,
};

// The `ifname` module's re-exports. Keep every `pub use ifname::...` line
// inside this region so it stays one contiguous block.
pub use ifname::{
    BRIDGE_TAG, DEFAULT_PREFIX, DerivedRole, HASH_SUFFIX_LEN, IfName, IfNameError, IfNameMapping,
    MAX_IFNAME_BYTES, NetworkIfRole, TAP_TAG, derive_from_env_vm, derive_ifname, detect_collisions,
    looks_d2b_owned, validate_prefix,
};

// The `credential_controller` module's re-exports. Keep every
// `pub use credential_controller::...` line inside this region so it stays one
// contiguous block.
pub use credential_controller::{
    CREDENTIAL_METRICS, CREDENTIAL_OBSERVE_INTERVAL_MS, CREDENTIAL_PROVIDER_REVOKE_FINALIZER,
    CredentialAuditDigest, CredentialAuditOperation, CredentialAuditOutcome, CredentialAuditRecord,
    CredentialControllerCall, CredentialControllerConditions, CredentialControllerDecision,
    CredentialControllerDisposition, CredentialControllerError, CredentialControllerHandlers,
    CredentialControllerHealth, CredentialControllerHealthState, CredentialControllerOutcome,
    CredentialIdempotencyKey, CredentialLeaseAggregate, CredentialMetricDescriptor,
    CredentialMetricKind, CredentialObservabilityError, CredentialObserveInput,
    CredentialProviderKind, CredentialReconcileInput, CredentialRetryState,
    CredentialRevocationInput, CredentialSingleFlight, CredentialSingleFlightGuard,
    CredentialTelemetryField, CredentialTelemetryFrame, CredentialTelemetryOperation,
    CredentialTelemetryOutcome, MAX_LOCAL_CREDENTIAL_LEASES, contains_sensitive_shape,
    observe_credential, owner_delete_action, provider_generation_action, reconcile_credential,
    revoke_credential,
};
