//! Canonical standard resource-plane contracts.

pub mod activation_nixos;
pub mod artifact;
pub mod authority;
pub mod binding;
pub mod bridge;
pub mod credential_binding;
pub mod device;
pub mod device_binding;
pub mod endpoint_binding;
pub mod error;
pub mod execution_policy;
pub mod execution_policy_resource;
pub mod host;
pub mod identity;
pub mod limits;
pub mod network;
pub mod network_binding;
pub mod operation;
pub mod operations;
pub mod payload_schema;
pub mod process;
pub mod resource;
pub mod resource_schema;
pub mod resource_status;
pub mod seccomp_profile;
pub mod storage;
pub mod user;
pub mod volume;
pub mod volume_binding;
pub mod volume_state;

pub use activation_nixos::*;
pub use artifact::*;
pub use authority::{
    AdmissionDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind, DesiredDigest,
    DesiredRevision, FreshnessTuple, RefusalReason, RevisionError, StoreIncarnation,
    ZoneDesiredSequence, DESIRED_ROW_DIGEST_DOMAIN_TAG, MAX_STORE_INCARNATION_BYTES,
};
pub use binding::{
    admit_binding_row_refs, BindingRowError,
    admit_binding_request, BindingAdmission, BindingArbitration, BindingAuthorization,
    BindingConsumerKind, BindingContractError, BindingEvidence, BindingKey, BindingKind,
    BindingLifecycleState, BindingObservation, BindingRealizationFacet, BindingRealizationSupport,
    BindingRefusal, BindingSlot, BindingSlotAddress, BindingSlotDecision, BindingSlotEntry,
    BindingSlotIndex, BindingSpecFingerprint, BindingSupportEntry, ChildBindingRequest,
    ChildRequestDefaults, ChildSupportCeiling, CompletionCondition, DefaultedSource,
    ExecutionParentInput, ExecutionParentInputClass, ReleaseOutcome, RequestedRights,
    SourceAdmission, SourceReservation, MAX_BINDING_DEPENDENCIES, MAX_BINDING_SLOT_BYTES,
    MAX_BINDING_SUPPORT_ENTRIES, MAX_CONSUMER_DEVICE_SLOT,
};
pub use bridge::*;
pub use credential_binding::{
    CredentialBindingRequest, CredentialLifetime, CredentialOperation,
    CREDENTIAL_BINDING_RESOURCE_TYPE, MAX_CREDENTIAL_LIFETIME_MS, MAX_CREDENTIAL_OPERATIONS,
    MIN_CREDENTIAL_LIFETIME_MS,
};
pub use device::*;
pub use device_binding::{
    DeviceAttachmentMode, DeviceBindingRequest, DeviceClaimRequest, DeviceFunction,
    DEVICE_BINDING_RESOURCE_TYPE,
};
pub use endpoint_binding::{
    EndpointAttachmentKind, EndpointBindingRequest, ENDPOINT_BINDING_RESOURCE_TYPE,
};
pub use error::{
    MAX_RESOURCE_ERROR_REASON_BYTES, MAX_RESOURCE_ERROR_RETRY_AFTER_MS, ResourceError,
    ResourceErrorKind, ResourceErrorReason, ResourceErrorValidation, RetryClass,
};
pub use execution_policy::*;
pub use host::*;
pub use execution_policy_resource::{
    AdmittedExecution, BackendSupport, BudgetCeiling, BudgetRequest, ConfinementFacet,
    ExecutionInstance, ExecutionInstanceKind, ExecutionPolicyFingerprint, ExecutionPolicySpec,
    ExecutionRequirements, PolicyAuthorization, PolicyCapabilities, PolicyContractError,
    PolicyIdentity, PolicyNamespaces, PolicyRefusal, PolicyRoot, PolicySeccomp,
    ALL_CONFINEMENT_FACETS, EXECUTION_POLICY_PROVIDER_REF, EXECUTION_POLICY_RESOURCE_TYPE,
    MAX_POLICY_CAPABILITY_CLASSES, MAX_POLICY_FDS, MAX_POLICY_MEMORY_BYTES, MAX_POLICY_MILLICPU,
    MAX_POLICY_NAMESPACE_CLASSES, MAX_POLICY_PIDS, MAX_POLICY_UMASK, admit_execution,
};
pub use identity::{
    ConfigurationGeneration, ControllerGeneration, IdentityClass, IdentityError,
    ObservedGeneration, ResourceBundleGenerationId, ResourceGeneration, ResourceName,
    ResourceTypeName, ResourceUid, SchemaFingerprint, Timestamp, V3_CONVERTED_RESOURCE_TYPES,
    ZoneId, ZoneResourceIdentity, ZoneRevision,
};
pub mod ifname {
    pub use d2b_contracts::v3::ifname::*;
}
pub use d2b_contracts::identity::ResourceRef;
pub use ifname::*;
pub use limits::*;
pub use network::*;
pub use network_binding::{
    NetworkBindingRequest, NetworkMembership, NetworkPresentation, NETWORK_BINDING_RESOURCE_TYPE,
};
pub use operation::{
    AuditJoin, AuditMode, BrokerRequirement, CallableOperation, FdContract, FdKind,
    OperationAudit, OperationAuthority, OperationBounds, OperationContractError, OperationDomain,
    OperationFds, OperationImplementation, OperationSurface, PayloadProvenance, PreopenedFd,
    SecretAccess, MAX_OPERATION_AUDIT_FIELDS, MAX_OPERATION_BATCH_ENTRIES, MAX_OPERATION_FDS,
    MAX_OPERATION_JOIN_FIELDS, MAX_OPERATION_PAYLOAD_BYTES, MAX_OPERATION_REDACTION_KEYS,
    MAX_OPERATION_STREAM_CREDITS, OPERATION_RESOURCE_TYPE, PROVIDER_RESOURCE_TYPE,
};
pub use operations::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    MAX_STORE_SLOTS, MutationOrdinal, MutationOrdinalError, MutationSealAcceptor, MutationSealBody,
    MutationSealIssuer, OpenedMutation, PolicySnapshot, PreparedStoreMutation,
    ResourceAssignmentFence, ResourceAssignmentScope, ResourceMutationKind, SealIdentityMismatch,
    SealedMutation, StoreCommitResult, StoreError, StoreErrorKind, StoreFilter, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreMutation,
    StoreOperationContext, StoreProjection, StoreResolveRequest, StoreResolvedIdentity,
    StoreSealIdentity, StoreSlot, StoreSlotError, StoreWatchReceipt, StoreWatchRequest,
    StoredResource, StoredSchema,
};
pub use payload_schema::*;
pub use seccomp_profile::{
    SeccompContractError, SeccompProfileSpec, SyscallDefaultAction, SyscallFilter,
    MAX_SECCOMP_SYSCALLS, SECCOMP_PROFILE_RESOURCE_TYPE,
};
pub use process::*;
pub use resource::{
    DisruptiveUpdateMode, FinalizerId, ManagedBy, NonDisruptiveUpdateMode, PresentationMetadata,
    ProviderSpecExtension, ResourceEnvelope, ResourceError as ResourceObjectError,
    ResourceMetadata, ResourceSpec, UpdatePolicy,
};
pub use resource_schema::*;
pub use resource_status::*;
pub use storage::*;
pub use user::*;
pub use volume_binding::*;
pub use volume_state::*;
