//! Storage-neutral resource store contract.
//!
//! This crate intentionally contains no database or executor dependency.

pub mod error;

use std::future::Future;

use d2b_contracts::v3::{
    ConfigurationGeneration, ControllerGeneration, FinalizerId, ResourceGeneration, ResourceName,
    ResourceRef, ResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
};

pub use error::{MutationOrdinal, MutationOrdinalError, StoreError, StoreErrorKind};

/// Exact optimistic precondition for a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedRevision {
    CreateAbsent,
    Exact(ZoneRevision),
}

/// Status projection selected before reading a resource body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreProjection {
    Full,
    BaseOnly,
    MetadataOnly,
}

/// Exact-match indexed filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreFilter {
    pub field: String,
    pub values: Vec<String>,
}

/// One resource body returned by the store.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredResource {
    pub resource_ref: ResourceRef,
    pub zone: ZoneId,
    pub uid: ResourceUid,
    pub generation: ResourceGeneration,
    pub revision: ZoneRevision,
    pub canonical_json: Vec<u8>,
    pub payload_digest: String,
}

impl core::fmt::Debug for StoredResource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoredResource")
            .field("resource_ref", &self.resource_ref)
            .field("zone", &self.zone)
            .field("uid", &self.uid)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field(
                "canonical_json",
                &format_args!("<{} bytes>", self.canonical_json.len()),
            )
            .field("payload_digest", &self.payload_digest)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreGetRequest {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub target: ResourceRef,
    pub expected_uid: Option<ResourceUid>,
    pub projection: StoreProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreListRequest {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub resource_types: Vec<ResourceTypeName>,
    pub resource_names: Vec<ResourceName>,
    pub filters: Vec<StoreFilter>,
    pub page_size: u32,
    pub cursor: Option<String>,
    pub projection: StoreProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreListResult {
    pub resources: Vec<StoredResource>,
    pub snapshot_revision: ZoneRevision,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreWatchRequest {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub resource_types: Vec<ResourceTypeName>,
    pub resource_names: Vec<ResourceName>,
    pub filters: Vec<StoreFilter>,
    pub after_revision: ZoneRevision,
    pub initial_credits: u32,
    pub projection: StoreProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreWatchReceipt {
    pub stream_name: String,
    pub snapshot_revision: ZoneRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreResolveRequest {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub target: ResourceRef,
    pub expected_uid: Option<ResourceUid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreResolvedIdentity {
    pub zone: ZoneId,
    pub resource_ref: ResourceRef,
    pub uid: ResourceUid,
    pub generation: ResourceGeneration,
    pub revision: ZoneRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInspectSchemaRequest {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub resource_type: ResourceTypeName,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StoredSchema {
    pub resource_type: ResourceTypeName,
    pub canonical_json: Vec<u8>,
    pub payload_digest: String,
}

impl core::fmt::Debug for StoredSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoredSchema")
            .field("resource_type", &self.resource_type)
            .field(
                "canonical_json",
                &format_args!("<{} bytes>", self.canonical_json.len()),
            )
            .field("payload_digest", &self.payload_digest)
            .finish()
    }
}

/// One full replacement mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMutationKind {
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
}

/// Structurally decoded mutation; authorization is attached only by admission.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreMutation {
    pub kind: ResourceMutationKind,
    pub zone: ZoneId,
    pub target: ResourceRef,
    pub expected: ExpectedRevision,
    pub expected_uid: Option<ResourceUid>,
    pub owner: Option<ResourceRef>,
    pub canonical_resource: Option<Vec<u8>>,
    pub add_finalizers: Vec<FinalizerId>,
    pub remove_finalizers: Vec<FinalizerId>,
    pub wait_for_reconcile: bool,
    pub reconcile_deadline_ms: Option<u64>,
}

impl core::fmt::Debug for StoreMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoreMutation")
            .field("kind", &self.kind)
            .field("zone", &self.zone)
            .field("target", &self.target)
            .field("expected", &self.expected)
            .field("owner", &self.owner)
            .field(
                "canonical_resource",
                &self.canonical_resource.as_ref().map(Vec::len),
            )
            .field("add_finalizers", &self.add_finalizers)
            .field("remove_finalizers", &self.remove_finalizers)
            .field("wait_for_reconcile", &self.wait_for_reconcile)
            .finish()
    }
}

/// Closed operation admitted by the evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmittedVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
    UseCredential,
    AdminCredential,
}

/// Exact target attributes evaluated before a mutation was queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedAuthorizationTarget {
    pub resource_type: ResourceTypeName,
    pub resource_name: Option<ResourceName>,
    pub verb: AdmittedVerb,
    pub subresource: Option<String>,
    pub execution_ref: Option<ResourceRef>,
}

/// Exact authenticated and target attributes captured at admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedAuthorization {
    pub zone: ZoneId,
    pub subject_ref: ResourceRef,
    pub subject_uid: ResourceUid,
    pub targets: Vec<AdmittedAuthorizationTarget>,
}

/// Revisions that the write transaction must compare for equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub policy_revision: u64,
    pub api_catalog_revision: u64,
    pub active_configuration_revision: ConfigurationGeneration,
    pub controller_generation: Option<ControllerGeneration>,
}

/// Fixed operation metadata captured before queueing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreOperationContext {
    pub operation_id: String,
    pub idempotency_key: Option<String>,
    pub correlation_id: String,
    pub trace_id: Option<String>,
    pub deadline_ms: u64,
}

mod admission {
    use super::{AdmittedAuthorization, PolicySnapshot, StoreMutation, StoreOperationContext};

    /// Witness produced only after the native evaluator returns allow.
    ///
    /// External code cannot mint the witness:
    ///
    /// ```compile_fail
    /// use d2b_resource_store::AllowDecision;
    ///
    /// let _forged = AllowDecision::from_native_evaluator();
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AllowDecision {
        private: (),
    }

    impl AllowDecision {
        const fn from_native_evaluator() -> Self {
            Self { private: () }
        }
    }

    /// Mutation value accepted by the store boundary.
    ///
    /// External code cannot attach arbitrary request state to an allow witness:
    ///
    /// ```compile_fail
    /// use d2b_resource_store::AdmittedMutation;
    ///
    /// let _forged = AdmittedMutation::new;
    /// ```
    #[derive(Clone, PartialEq, Eq)]
    pub struct AdmittedMutation {
        mutations: Vec<StoreMutation>,
        authorization: AdmittedAuthorization,
        policy_snapshot: PolicySnapshot,
        operation: StoreOperationContext,
        allow: AllowDecision,
    }

    impl AdmittedMutation {
        fn new(
            mutations: Vec<StoreMutation>,
            authorization: AdmittedAuthorization,
            policy_snapshot: PolicySnapshot,
            operation: StoreOperationContext,
            allow: AllowDecision,
        ) -> Self {
            Self {
                mutations,
                authorization,
                policy_snapshot,
                operation,
                allow,
            }
        }

        pub fn mutations(&self) -> &[StoreMutation] {
            &self.mutations
        }

        pub const fn authorization(&self) -> &AdmittedAuthorization {
            &self.authorization
        }

        pub const fn policy_snapshot(&self) -> PolicySnapshot {
            self.policy_snapshot
        }

        pub const fn operation(&self) -> &StoreOperationContext {
            &self.operation
        }
    }

    impl core::fmt::Debug for AdmittedMutation {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("AdmittedMutation")
                .field("mutations", &self.mutations)
                .field("authorization", &self.authorization)
                .field("policy_snapshot", &self.policy_snapshot)
                .field("operation", &self.operation)
                .field("allow", &"<allow>")
                .finish()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use d2b_contracts::v3::{
            ConfigurationGeneration, ResourceName, ResourceRef, ResourceTypeName, ResourceUid,
            ZoneId,
        };

        #[test]
        fn evaluator_boundary_can_mint_admitted_mutation() {
            let allow = AllowDecision::from_native_evaluator();
            let admitted = AdmittedMutation::new(
                Vec::new(),
                AdmittedAuthorization {
                    zone: ZoneId::parse("work").unwrap(),
                    subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
                    subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                        .unwrap(),
                    targets: vec![super::super::AdmittedAuthorizationTarget {
                        resource_type: ResourceTypeName::parse("Host").unwrap(),
                        resource_name: Some(ResourceName::parse("local").unwrap()),
                        verb: super::super::AdmittedVerb::Create,
                        subresource: None,
                        execution_ref: None,
                    }],
                },
                PolicySnapshot {
                    policy_revision: 1,
                    api_catalog_revision: 1,
                    active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                    controller_generation: None,
                },
                StoreOperationContext {
                    operation_id: "op-1".to_owned(),
                    idempotency_key: None,
                    correlation_id: "correlation-1".to_owned(),
                    trace_id: None,
                    deadline_ms: 1,
                },
                allow,
            );

            assert_eq!(admitted.mutations(), &[]);
            assert_eq!(admitted.policy_snapshot().policy_revision, 1);
        }
    }
}

pub use admission::{AdmittedMutation, AllowDecision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCommitResult {
    pub resources: Vec<StoredResource>,
    pub revision: ZoneRevision,
}

/// Runtime-neutral async store interface consumed only by the resource API.
pub trait ResourceStore: Send + Sync {
    fn get(
        &self,
        request: StoreGetRequest,
    ) -> impl Future<Output = Result<StoredResource, StoreError>> + Send;

    fn list(
        &self,
        request: StoreListRequest,
    ) -> impl Future<Output = Result<StoreListResult, StoreError>> + Send;

    fn watch(
        &self,
        request: StoreWatchRequest,
    ) -> impl Future<Output = Result<StoreWatchReceipt, StoreError>> + Send;

    fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> impl Future<Output = Result<StoreResolvedIdentity, StoreError>> + Send;

    fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> impl Future<Output = Result<StoredSchema, StoreError>> + Send;

    fn commit(
        &self,
        mutation: AdmittedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send;
}
