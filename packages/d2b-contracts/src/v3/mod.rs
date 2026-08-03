//! Canonical d2b v3 resource-plane contracts.

pub mod component_session;
pub mod credential;
pub mod credential_controller;
pub mod device;
pub mod emergency_policy;
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
pub mod provider_registry;
pub mod quota;
pub mod resource;
pub mod resource_bundle;
pub mod resource_export;
pub mod resource_import;
pub mod resource_ref;
pub mod resource_schema;
pub mod resource_status;
pub mod role;
pub mod role_binding;
pub mod semantic_services;
pub mod services;
pub mod storage;
pub mod telemetry_policy;
pub mod user;
pub mod volume;
pub mod volume_state;
pub mod zone;
pub mod zone_link;
pub mod zone_routing;
pub mod zone_session;

pub use emergency_policy::{
    EMERGENCY_DRAIN_FINALIZER, EMERGENCY_POLICY_RESOURCE_TYPE, EmergencyPolicyConditionType,
    EmergencyPolicySpec, EmergencyPolicyStatus, EmergencyPolicyStatusResource, EmergencyScope,
    effective_scope,
};
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
pub use resource_export::{
    ConsumerZonePolicy, ExportArbitration, ExportLeaseState, ExportLeaseSummary, ExportVisibility,
    Fairness, MAX_RESOURCE_EXPORT_CAPABILITIES, MAX_RESOURCE_EXPORT_CONSUMERS,
    MAX_RESOURCE_EXPORT_LEASE_DEADLINE_MS, MAX_RESOURCE_EXPORT_LEASE_SUMMARIES,
    MAX_RESOURCE_EXPORT_OPERATIONS, MAX_RESOURCE_EXPORT_RATE,
    MAX_RESOURCE_EXPORT_REVOCATION_GRACE_MS, MAX_RESOURCE_EXPORT_ZONES,
    RESOURCE_EXPORT_DRAIN_FINALIZER, RESOURCE_EXPORT_RESOURCE_TYPE, ResourceExportConditionType,
    ResourceExportContractError, ResourceExportError, ResourceExportSpec, ResourceExportState,
    ResourceExportStatus, ResourceExportStatusResource, RevocationPolicy, ShareFairness,
    ShareQuota,
};
pub use resource_import::{
    ImportDisconnectPolicy, MAX_RESOURCE_IMPORT_CAPABILITIES, MAX_RESOURCE_IMPORT_EXPORT_KEY_BYTES,
    MAX_RESOURCE_IMPORT_LEASE_COUNT, RESOURCE_IMPORT_DRAIN_FINALIZER,
    RESOURCE_IMPORT_RESOURCE_TYPE, ResourceImportConditionType, ResourceImportContractError,
    ResourceImportError, ResourceImportSpec, ResourceImportState, ResourceImportStatus,
    ResourceImportStatusResource,
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
pub use provider_registry::{
    MAX_PROVIDER_MAPPING_ID_BYTES, MAX_PROVIDER_REGISTRY_MAPPINGS, ProviderBindingAxis,
    ProviderRegistryEntry, ProviderRegistryPublication,
};
pub use quota::{
    MAX_QUOTA_OWNER_DEPTH, MAX_QUOTA_PER_TYPE_ENTRIES, MAX_QUOTA_RESOURCES, QUOTA_DRAIN_FINALIZER,
    QUOTA_RESOURCE_TYPE, QuotaCeilings, QuotaEnforcementPolicy, QuotaScope, QuotaSpec, QuotaStatus,
    QuotaStatusResource, QuotaTypeCeiling,
};
pub use role::{
    RoleConditionType, RoleResourceVerb, RoleRule, RoleSessionVerb, RoleSpec, RoleStatus,
    RoleStatusResource,
};
pub use role_binding::{
    ExternalPrincipalSelector, RoleBindingConditionType, RoleBindingSpec, RoleBindingStatus,
    RoleBindingStatusResource, ScopeNarrowing,
};
pub use services::{
    AuditSegment, ProviderMethod, ResourceMethod, ServiceDescriptor, ServiceDescriptorError,
    V3Service, ZoneMethod, missing_audit_segments,
};
pub use telemetry_policy::{
    FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, METRIC_LABEL_POLICY, OTEL_RESOURCE_ATTRIBUTES,
    allowed_values as telemetry_allowed_values,
};
pub use zone::{
    ZoneConditionType, ZoneHandlerName, ZoneHandlerPhase, ZoneHandlerStatus, ZoneSpec, ZoneStatus,
    ZoneStatusResource, validate_finalizer, validate_self_resource,
};
pub use zone_link::{
    ZoneLinkConditionType, ZoneLinkLimits, ZoneLinkSpec, ZoneLinkStatus, ZoneLinkStatusResource,
    admit_local_intent,
};

// The `semantic_services` module's re-exports. Keep every
// `pub use semantic_services::...` line inside this region so it stays one
// contiguous block.
pub use semantic_services::{
    LEGACY_ABSENT_PROTOCOL_VERSION, PROVIDER_REF_FIELD, SEMANTIC_BASE_SCHEMA_MAJOR,
    SEMANTIC_BASE_SCHEMA_MINOR, SEMANTIC_PROJECTION_PROTOCOL_VERSION, SemanticContractError,
    SemanticFamily, SemanticLayer, SemanticLayerSchema, SemanticPairContract,
    SemanticProjectionBinding, SemanticProjectionProtocolVersion, SemanticRole, SemanticSchemaId,
    SemanticTypeContract, UPDATE_POLICY_FIELD, catalog as semantic_service_catalog,
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
