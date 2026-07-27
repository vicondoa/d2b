//! Storage-neutral resource store contract.
//!
//! This crate intentionally contains no database or executor dependency.

pub mod error;

use std::future::Future;

use d2b_contracts::v3::{
    ConfigurationGeneration, ControllerGeneration, FinalizerId, ResourceGeneration, ResourceName,
    ResourceRef, ResourceTypeName, ResourceUid, RetryClass, ZoneId, ZoneRevision,
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
    use super::{
        AdmittedAuthorization, MutationOrdinal, PolicySnapshot, RetryClass, StoreError,
        StoreErrorKind, StoreMutation, StoreOperationContext,
    };
    use std::sync::Arc;

    #[derive(Debug)]
    struct AdmissionAuthority;

    /// Capability installed only in the native evaluator.
    #[derive(Clone)]
    pub struct AdmissionIssuer {
        authority: Arc<AdmissionAuthority>,
    }

    impl core::fmt::Debug for AdmissionIssuer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("AdmissionIssuer(<redacted>)")
        }
    }

    /// Store-side half of an instance-bound admission capability.
    pub struct AdmissionVerifier {
        authority: Arc<AdmissionAuthority>,
    }

    impl core::fmt::Debug for AdmissionVerifier {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("AdmissionVerifier(<redacted>)")
        }
    }

    /// Create the evaluator capability and its store verifier.
    pub fn admission_pair() -> (AdmissionIssuer, AdmissionVerifier) {
        let authority = Arc::new(AdmissionAuthority);
        (
            AdmissionIssuer {
                authority: Arc::clone(&authority),
            },
            AdmissionVerifier { authority },
        )
    }

    /// Positive evaluation result carrying the exact authorization and revisions.
    pub struct AdmissionPermit {
        authority: Arc<AdmissionAuthority>,
        authorization: AdmittedAuthorization,
        policy_snapshot: PolicySnapshot,
    }

    impl core::fmt::Debug for AdmissionPermit {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("AdmissionPermit")
                .field("authorization", &self.authorization)
                .field("policy_snapshot", &self.policy_snapshot)
                .field("authority", &"<redacted>")
                .finish()
        }
    }

    impl AdmissionIssuer {
        /// Capture one allow returned by the evaluator that owns this capability.
        pub fn record_allow(
            &self,
            authorization: AdmittedAuthorization,
            policy_snapshot: PolicySnapshot,
        ) -> AdmissionPermit {
            AdmissionPermit {
                authority: Arc::clone(&self.authority),
                authorization,
                policy_snapshot,
            }
        }
    }

    /// Admission failed before the store backend was reachable.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AdmissionError {
        ZoneMismatch(MutationOrdinal),
    }

    impl core::fmt::Display for AdmissionError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::ZoneMismatch(_) => f.write_str("mutation Zone does not match admission"),
            }
        }
    }

    impl std::error::Error for AdmissionError {}

    impl AdmissionPermit {
        /// Bind parsed mutations to this exact positive evaluation result.
        pub fn admit(
            self,
            mutations: Vec<StoreMutation>,
            operation: StoreOperationContext,
        ) -> Result<AdmittedMutation, AdmissionError> {
            for (ordinal, mutation) in mutations.iter().enumerate() {
                if mutation.zone != self.authorization.zone {
                    let ordinal = MutationOrdinal::new(
                        u32::try_from(ordinal)
                            .expect("a bounded mutation batch ordinal always fits u32"),
                    )
                    .expect("the API rejects mutation batches above the frozen bound");
                    return Err(AdmissionError::ZoneMismatch(ordinal));
                }
            }
            Ok(AdmittedMutation {
                mutations,
                authorization: self.authorization,
                policy_snapshot: self.policy_snapshot,
                operation,
                authority: self.authority,
            })
        }
    }

    /// Mutation value accepted by the checked store boundary.
    ///
    /// External code cannot construct an admission witness:
    ///
    /// ```compile_fail
    /// use d2b_resource_store::AdmittedMutation;
    ///
    /// let _forged = AdmittedMutation::new;
    /// ```
    pub struct AdmittedMutation {
        mutations: Vec<StoreMutation>,
        authorization: AdmittedAuthorization,
        policy_snapshot: PolicySnapshot,
        operation: StoreOperationContext,
        authority: Arc<AdmissionAuthority>,
    }

    impl AdmittedMutation {
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
                .field("authority", &"<redacted>")
                .finish()
        }
    }

    /// Mutation whose admission authority was matched to the store instance.
    pub struct VerifiedMutation {
        admitted: AdmittedMutation,
    }

    impl VerifiedMutation {
        pub fn mutations(&self) -> &[StoreMutation] {
            self.admitted.mutations()
        }

        pub const fn authorization(&self) -> &AdmittedAuthorization {
            self.admitted.authorization()
        }

        pub const fn policy_snapshot(&self) -> PolicySnapshot {
            self.admitted.policy_snapshot()
        }

        pub const fn operation(&self) -> &StoreOperationContext {
            self.admitted.operation()
        }
    }

    impl core::fmt::Debug for VerifiedMutation {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_tuple("VerifiedMutation")
                .field(&self.admitted)
                .finish()
        }
    }

    impl AdmissionVerifier {
        pub(super) fn verify(
            &self,
            admitted: AdmittedMutation,
        ) -> Result<VerifiedMutation, StoreError> {
            if !Arc::ptr_eq(&self.authority, &admitted.authority) {
                return Err(StoreError::new(
                    StoreErrorKind::InternalIntegrityFailure,
                    None,
                    None,
                    RetryClass::Never,
                    "admission-authority-mismatch",
                ));
            }
            Ok(VerifiedMutation { admitted })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use d2b_contracts::v3::{
            ConfigurationGeneration, ResourceName, ResourceRef, ResourceTypeName, ResourceUid,
            ZoneId,
        };

        fn authorization(zone: &str) -> AdmittedAuthorization {
            AdmittedAuthorization {
                zone: ZoneId::parse(zone).unwrap(),
                subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
                subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                targets: vec![super::super::AdmittedAuthorizationTarget {
                    resource_type: ResourceTypeName::parse("Host").unwrap(),
                    resource_name: Some(ResourceName::parse("local").unwrap()),
                    verb: super::super::AdmittedVerb::Create,
                    subresource: None,
                    execution_ref: None,
                }],
            }
        }

        fn snapshot() -> PolicySnapshot {
            PolicySnapshot {
                policy_revision: 1,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                controller_generation: None,
            }
        }

        fn operation() -> StoreOperationContext {
            StoreOperationContext {
                operation_id: "op-1".to_owned(),
                idempotency_key: None,
                correlation_id: "correlation-1".to_owned(),
                trace_id: None,
                deadline_ms: 1,
            }
        }

        fn mutation(zone: &str) -> StoreMutation {
            StoreMutation {
                kind: super::super::ResourceMutationKind::Create,
                zone: ZoneId::parse(zone).unwrap(),
                target: ResourceRef::parse("Host/local").unwrap(),
                expected: super::super::ExpectedRevision::CreateAbsent,
                expected_uid: None,
                owner: None,
                canonical_resource: None,
                add_finalizers: Vec::new(),
                remove_finalizers: Vec::new(),
                wait_for_reconcile: false,
                reconcile_deadline_ms: None,
            }
        }

        #[test]
        fn evaluator_capability_mints_store_bound_admission() {
            let (issuer, verifier) = admission_pair();
            let permit = issuer.record_allow(authorization("work"), snapshot());
            let admitted = permit
                .admit(Vec::new(), operation())
                .expect("matching admission");
            let verified = verifier.verify(admitted).expect("paired verifier");

            assert_eq!(verified.mutations(), &[]);
            assert_eq!(verified.policy_snapshot().policy_revision, 1);
        }

        #[test]
        fn mixed_zone_admission_is_impossible() {
            let (issuer, _) = admission_pair();
            let permit = issuer.record_allow(authorization("work"), snapshot());
            let error = permit
                .admit(vec![mutation("work"), mutation("personal")], operation())
                .unwrap_err();

            assert_eq!(
                error,
                AdmissionError::ZoneMismatch(MutationOrdinal::new(1).unwrap())
            );
        }

        #[test]
        fn verifier_rejects_admission_from_another_store_instance() {
            let (issuer, _) = admission_pair();
            let (_, other_verifier) = admission_pair();
            let admitted = issuer
                .record_allow(authorization("work"), snapshot())
                .admit(Vec::new(), operation())
                .unwrap();

            let error = other_verifier.verify(admitted).unwrap_err();
            assert_eq!(error.kind(), StoreErrorKind::InternalIntegrityFailure);
            assert_eq!(error.reason_code(), "admission-authority-mismatch");
        }

        #[test]
        fn evaluation_snapshot_cannot_be_substituted_during_admission() {
            let (issuer, verifier) = admission_pair();
            let admitted = issuer
                .record_allow(authorization("work"), snapshot())
                .admit(Vec::new(), operation())
                .unwrap();
            let verified = verifier.verify(admitted).unwrap();

            assert_eq!(verified.policy_snapshot(), snapshot());
        }
    }
}

pub use admission::{
    AdmissionError, AdmissionIssuer, AdmissionPermit, AdmissionVerifier, AdmittedMutation,
    VerifiedMutation, admission_pair,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCommitResult {
    pub resources: Vec<StoredResource>,
    pub revision: ZoneRevision,
}

/// Backend seam reached only after instance-bound admission verification.
pub trait ResourceStoreBackend: Send + Sync {
    fn admission_verifier(&self) -> &AdmissionVerifier;

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

    fn commit_verified(
        &self,
        mutation: VerifiedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send;
}

mod sealed {
    pub trait Sealed {}

    impl<T: super::ResourceStoreBackend> Sealed for T {}
}

/// Runtime-neutral store interface with a non-bypassable admission check.
pub trait ResourceStore: sealed::Sealed + Send + Sync {
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

impl<T: ResourceStoreBackend> ResourceStore for T {
    fn get(
        &self,
        request: StoreGetRequest,
    ) -> impl Future<Output = Result<StoredResource, StoreError>> + Send {
        ResourceStoreBackend::get(self, request)
    }

    fn list(
        &self,
        request: StoreListRequest,
    ) -> impl Future<Output = Result<StoreListResult, StoreError>> + Send {
        ResourceStoreBackend::list(self, request)
    }

    fn watch(
        &self,
        request: StoreWatchRequest,
    ) -> impl Future<Output = Result<StoreWatchReceipt, StoreError>> + Send {
        ResourceStoreBackend::watch(self, request)
    }

    fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> impl Future<Output = Result<StoreResolvedIdentity, StoreError>> + Send {
        ResourceStoreBackend::resolve_ref(self, request)
    }

    fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> impl Future<Output = Result<StoredSchema, StoreError>> + Send {
        ResourceStoreBackend::inspect_schema(self, request)
    }

    fn commit(
        &self,
        mutation: AdmittedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send {
        let verified = self.admission_verifier().verify(mutation);
        async move { ResourceStoreBackend::commit_verified(self, verified?).await }
    }
}
