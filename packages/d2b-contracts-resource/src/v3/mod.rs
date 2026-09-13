//! Canonical standard resource-plane contracts.

pub mod activation_nixos;
pub mod artifact;
pub mod bridge;
pub mod device;
pub mod endpoint;
pub mod error;
pub mod execution_policy;
pub mod guest;
pub mod host;
pub mod identity;
pub mod limits;
pub mod network;
pub mod operations;
pub mod process;
pub mod quota;
pub mod resource;
pub mod resource_schema;
pub mod resource_status;
pub mod storage;
pub mod user;
pub mod volume;
pub mod volume_binding;
pub mod volume_state;

pub use activation_nixos::*;
pub use artifact::*;
pub use bridge::*;
pub use device::*;
pub use endpoint::*;
pub use error::{
    MAX_RESOURCE_ERROR_REASON_BYTES, MAX_RESOURCE_ERROR_RETRY_AFTER_MS, ResourceError,
    ResourceErrorKind, ResourceErrorReason, ResourceErrorValidation, RetryClass,
};
pub use execution_policy::*;
pub use guest::*;
pub use host::*;
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
pub use process::*;
pub use quota::*;
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
