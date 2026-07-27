//! Instance-bound admission witnesses owned by the native evaluator.

use d2b_contracts::v3::{
    CanonicalJsonValue, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceEnvelope, ResourceUid, RetryClass,
    canonical_digest,
};
use d2b_resource_store::{
    AdmittedAuthorization, ExpectedRevision, MutationOrdinal, PolicySnapshot, ResourceMutationKind,
    StoreError, StoreErrorKind, StoreMutation, StoreOperationContext,
};
use std::{fmt::Write as _, fs::File, io::Read, sync::Arc};

#[derive(Debug)]
struct AdmissionAuthority;

#[derive(Debug)]
struct StoreIdentityAuthority;

/// Capability installed only in the native evaluator.
pub(crate) struct AdmissionIssuer {
    authority: Arc<AdmissionAuthority>,
    store_identity: Arc<StoreIdentityAuthority>,
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

/// Unique identity owned by one concrete resource-store backend.
pub struct StoreIdentity {
    authority: Arc<StoreIdentityAuthority>,
}

impl core::fmt::Debug for StoreIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StoreIdentity(<redacted>)")
    }
}

/// Create the evaluator capability and its store verifier.
pub(crate) fn admission_pair() -> (AdmissionIssuer, AdmissionVerifier, StoreIdentity) {
    let authority = Arc::new(AdmissionAuthority);
    let store_identity = Arc::new(StoreIdentityAuthority);
    (
        AdmissionIssuer {
            authority: Arc::clone(&authority),
            store_identity: Arc::clone(&store_identity),
        },
        AdmissionVerifier { authority },
        StoreIdentity {
            authority: store_identity,
        },
    )
}

/// Positive evaluation result carrying the exact authorization and revisions.
pub(crate) struct AdmissionPermit {
    authority: Arc<AdmissionAuthority>,
    store_identity: Arc<StoreIdentityAuthority>,
    authorization: AdmittedAuthorization,
    policy_snapshot: PolicySnapshot,
}

impl core::fmt::Debug for AdmissionPermit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdmissionPermit")
            .field("target_count", &self.authorization.targets.len())
            .field("authorization", &"<redacted>")
            .field("policy_snapshot", &"<redacted>")
            .field("authority", &"<redacted>")
            .field("store_identity", &"<redacted>")
            .finish()
    }
}

impl AdmissionIssuer {
    /// Capture one allow returned by the evaluator that owns this capability.
    pub(crate) fn record_allow(
        &self,
        authorization: AdmittedAuthorization,
        policy_snapshot: PolicySnapshot,
    ) -> AdmissionPermit {
        AdmissionPermit {
            authority: Arc::clone(&self.authority),
            store_identity: Arc::clone(&self.store_identity),
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
            store_identity: self.store_identity,
        })
    }
}

/// Mutation value accepted by the checked store boundary.
///
/// External code cannot construct an admission witness:
///
/// ```compile_fail
/// use d2b_resource_api::AdmittedMutation;
///
/// let _forged = AdmittedMutation::new;
/// ```
pub struct AdmittedMutation {
    mutations: Vec<StoreMutation>,
    authorization: AdmittedAuthorization,
    policy_snapshot: PolicySnapshot,
    operation: StoreOperationContext,
    authority: Arc<AdmissionAuthority>,
    store_identity: Arc<StoreIdentityAuthority>,
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
            .field("mutation_count", &self.mutations.len())
            .field("authorization", &"<redacted>")
            .field("policy_snapshot", &"<redacted>")
            .field("operation", &"<redacted>")
            .field("authority", &"<redacted>")
            .field("store_identity", &"<redacted>")
            .finish()
    }
}

/// Mutation whose admission authority was matched to the store instance.
pub struct VerifiedMutation {
    admitted: AdmittedMutation,
    mutations: Vec<PreparedStoreMutation>,
}

/// Backend-ready mutation carrying the final canonical identity and digest.
pub struct PreparedStoreMutation {
    mutation: StoreMutation,
    resource_uid: Option<ResourceUid>,
    payload_digest: Option<String>,
}

impl PreparedStoreMutation {
    pub const fn mutation(&self) -> &StoreMutation {
        &self.mutation
    }

    /// Final UID used by the resource record and every UID-keyed index.
    pub const fn resource_uid(&self) -> Option<&ResourceUid> {
        self.resource_uid.as_ref()
    }

    /// Digest of the final canonical bytes persisted by the backend.
    pub fn payload_digest(&self) -> Option<&str> {
        self.payload_digest.as_deref()
    }
}

impl core::fmt::Debug for PreparedStoreMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PreparedStoreMutation")
            .field("kind", &self.mutation.kind)
            .field("has_resource_uid", &self.resource_uid.is_some())
            .field("has_payload_digest", &self.payload_digest.is_some())
            .finish()
    }
}

impl VerifiedMutation {
    fn check_authority(
        &self,
        verifier: &AdmissionVerifier,
        store_identity: &StoreIdentity,
    ) -> Result<(), StoreError> {
        if !Arc::ptr_eq(&verifier.authority, &self.admitted.authority) {
            return Err(authority_mismatch());
        }
        if !Arc::ptr_eq(&store_identity.authority, &self.admitted.store_identity) {
            return Err(store_identity_mismatch());
        }
        Ok(())
    }

    pub fn mutations(
        &self,
        verifier: &AdmissionVerifier,
        store_identity: &StoreIdentity,
    ) -> Result<&[PreparedStoreMutation], StoreError> {
        self.check_authority(verifier, store_identity)?;
        Ok(&self.mutations)
    }

    pub fn authorization(
        &self,
        verifier: &AdmissionVerifier,
        store_identity: &StoreIdentity,
    ) -> Result<&AdmittedAuthorization, StoreError> {
        self.check_authority(verifier, store_identity)?;
        Ok(self.admitted.authorization())
    }

    pub fn policy_snapshot(
        &self,
        verifier: &AdmissionVerifier,
        store_identity: &StoreIdentity,
    ) -> Result<PolicySnapshot, StoreError> {
        self.check_authority(verifier, store_identity)?;
        Ok(self.admitted.policy_snapshot())
    }

    pub fn operation(
        &self,
        verifier: &AdmissionVerifier,
        store_identity: &StoreIdentity,
    ) -> Result<&StoreOperationContext, StoreError> {
        self.check_authority(verifier, store_identity)?;
        Ok(self.admitted.operation())
    }
}

impl core::fmt::Debug for VerifiedMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VerifiedMutation(<redacted>)")
    }
}

impl AdmissionVerifier {
    pub(super) fn verify(
        &self,
        admitted: AdmittedMutation,
        store_identity: &StoreIdentity,
    ) -> Result<VerifiedMutation, StoreError> {
        if !Arc::ptr_eq(&self.authority, &admitted.authority) {
            return Err(authority_mismatch());
        }
        if !Arc::ptr_eq(&store_identity.authority, &admitted.store_identity) {
            return Err(store_identity_mismatch());
        }
        let mutations = admitted
            .mutations
            .iter()
            .cloned()
            .map(prepare_mutation)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(VerifiedMutation {
            admitted,
            mutations,
        })
    }
}

fn authority_mismatch() -> StoreError {
    StoreError::new(
        StoreErrorKind::InternalIntegrityFailure,
        None,
        None,
        RetryClass::Never,
        "admission-authority-mismatch",
    )
}

fn store_identity_mismatch() -> StoreError {
    StoreError::new(
        StoreErrorKind::InternalIntegrityFailure,
        None,
        None,
        RetryClass::Never,
        "admission-store-identity-mismatch",
    )
}

fn prepare_mutation(mut mutation: StoreMutation) -> Result<PreparedStoreMutation, StoreError> {
    let (resource_uid, payload_digest) = match mutation.kind {
        ResourceMutationKind::Create => {
            if mutation.expected != ExpectedRevision::CreateAbsent
                || mutation.expected_uid.is_some()
            {
                return Err(preparation_error("create-identity-precondition-invalid"));
            }
            let source = mutation
                .canonical_resource
                .as_deref()
                .ok_or_else(|| preparation_error("create-resource-body-missing"))?;
            let (canonical, uid, digest) = finalize_create(source, &mutation)?;
            mutation.canonical_resource = Some(canonical);
            (Some(uid), Some(digest))
        }
        _ => match mutation.canonical_resource.as_deref() {
            Some(source) => {
                let envelope = ResourceEnvelope::from_json(source)
                    .map_err(|_| preparation_error("resource-envelope-invalid"))?;
                validate_envelope_identity(&envelope, &mutation)?;
                let canonical = envelope
                    .canonical_bytes()
                    .map_err(|_| preparation_error("resource-envelope-invalid"))?;
                let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
                let uid = envelope.metadata().uid().clone();
                mutation.canonical_resource = Some(canonical);
                (Some(uid), Some(digest))
            }
            None => (mutation.expected_uid.clone(), None),
        },
    };
    Ok(PreparedStoreMutation {
        mutation,
        resource_uid,
        payload_digest,
    })
}

fn finalize_create(
    source: &[u8],
    mutation: &StoreMutation,
) -> Result<(Vec<u8>, ResourceUid, String), StoreError> {
    let mut value = CanonicalJsonValue::parse(source)
        .map_err(|_| preparation_error("create-resource-body-invalid"))?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(preparation_error("create-resource-body-invalid"));
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(preparation_error("create-resource-metadata-missing"));
    };
    if metadata.contains_key("uid") {
        return Err(preparation_error("create-resource-uid-present"));
    }
    let uid = mint_resource_uid()?;
    metadata.insert(
        "uid".to_owned(),
        CanonicalJsonValue::String(uid.as_str().to_owned()),
    );
    let canonical = value.to_canonical_bytes();
    let envelope = ResourceEnvelope::from_json(&canonical)
        .map_err(|_| preparation_error("create-resource-body-invalid"))?;
    validate_envelope_identity(&envelope, mutation)?;
    if envelope.metadata().uid() != &uid {
        return Err(preparation_error("create-resource-uid-mismatch"));
    }
    let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    Ok((canonical, uid, digest))
}

fn validate_envelope_identity(
    envelope: &ResourceEnvelope,
    mutation: &StoreMutation,
) -> Result<(), StoreError> {
    if envelope.resource_type() != mutation.target.resource_type()
        || envelope.metadata().name() != mutation.target.name()
        || envelope.metadata().zone() != &mutation.zone
        || mutation
            .expected_uid
            .as_ref()
            .is_some_and(|expected| expected != envelope.metadata().uid())
    {
        return Err(preparation_error("resource-envelope-identity-mismatch"));
    }
    Ok(())
}

fn mint_resource_uid() -> Result<ResourceUid, StoreError> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|_| preparation_error("resource-uid-entropy-unavailable"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut rendered = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            rendered.push('-');
        }
        write!(&mut rendered, "{byte:02x}").expect("writing to a String cannot fail");
    }
    ResourceUid::parse(rendered).map_err(|_| preparation_error("resource-uid-mint-invalid"))
}

fn preparation_error(reason_code: &'static str) -> StoreError {
    StoreError::new(
        StoreErrorKind::ResourceSchemaInvalid,
        None,
        None,
        RetryClass::Never,
        reason_code,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{
        ConfigurationGeneration, ResourceName, ResourceRef, ResourceTypeName, ResourceUid, ZoneId,
        ZoneRevision,
    };

    fn authorization(zone: &str) -> AdmittedAuthorization {
        AdmittedAuthorization {
            zone: ZoneId::parse(zone).unwrap(),
            subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
            subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            targets: vec![d2b_resource_store::AdmittedAuthorizationTarget {
                resource_type: ResourceTypeName::parse("Host").unwrap(),
                resource_name: Some(ResourceName::parse("local").unwrap()),
                verb: d2b_resource_store::AdmittedVerb::Create,
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
            kind: d2b_resource_store::ResourceMutationKind::Create,
            zone: ZoneId::parse(zone).unwrap(),
            target: ResourceRef::parse("Host/local").unwrap(),
            expected: d2b_resource_store::ExpectedRevision::CreateAbsent,
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
        let (issuer, verifier, store_identity) = admission_pair();
        let permit = issuer.record_allow(authorization("work"), snapshot());
        let admitted = permit
            .admit(Vec::new(), operation())
            .expect("matching admission");
        let verified = verifier
            .verify(admitted, &store_identity)
            .expect("paired verifier");

        assert!(
            verified
                .mutations(&verifier, &store_identity)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            verified
                .policy_snapshot(&verifier, &store_identity)
                .unwrap()
                .policy_revision,
            1
        );
    }

    #[test]
    fn mixed_zone_admission_is_impossible() {
        let (issuer, _, _) = admission_pair();
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
    fn create_uid_cannot_enter_through_the_store_boundary() {
        let (issuer, verifier, store_identity) = admission_pair();
        let mut create = mutation("work");
        create.canonical_resource =
            Some(br#"{"metadata":{"uid":"123e4567-e89b-42d3-a456-426614174000"}}"#.to_vec());
        let admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(vec![create], operation())
            .unwrap();

        let error = verifier.verify(admitted, &store_identity).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::ResourceSchemaInvalid);
        assert_eq!(error.reason_code(), "create-resource-uid-present");
    }

    #[test]
    fn minted_resource_uids_are_canonical_uuid_v4_values() {
        let first = mint_resource_uid().unwrap();
        let second = mint_resource_uid().unwrap();

        assert_ne!(first, second);
        assert_eq!(first.as_str().as_bytes()[14], b'4');
        assert!(matches!(
            first.as_str().as_bytes()[19],
            b'8' | b'9' | b'a' | b'b'
        ));
    }

    #[test]
    fn verifier_rejects_admission_from_another_store_instance() {
        let (issuer, _, _) = admission_pair();
        let (_, other_verifier, other_store_identity) = admission_pair();
        let admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(Vec::new(), operation())
            .unwrap();

        let error = other_verifier
            .verify(admitted, &other_store_identity)
            .unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::InternalIntegrityFailure);
        assert_eq!(error.reason_code(), "admission-authority-mismatch");
    }

    #[test]
    fn verified_mutation_cannot_be_forwarded_to_another_backend() {
        let (issuer, verifier, store_identity) = admission_pair();
        let (_, other_verifier, other_store_identity) = admission_pair();
        let admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(Vec::new(), operation())
            .unwrap();
        let verified = verifier.verify(admitted, &store_identity).unwrap();

        let error = verified
            .mutations(&other_verifier, &other_store_identity)
            .unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::InternalIntegrityFailure);
        assert_eq!(error.reason_code(), "admission-authority-mismatch");
    }

    #[test]
    fn shared_verifier_cannot_cross_store_identities() {
        struct TestStore {
            verifier: Arc<AdmissionVerifier>,
            identity: StoreIdentity,
        }

        let (issuer, shared_verifier, first_identity) = admission_pair();
        let (_, _, second_identity) = admission_pair();
        let shared_verifier = Arc::new(shared_verifier);
        let first_store = TestStore {
            verifier: Arc::clone(&shared_verifier),
            identity: first_identity,
        };
        let second_store = TestStore {
            verifier: shared_verifier,
            identity: second_identity,
        };
        let first_admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(Vec::new(), operation())
            .unwrap();
        assert!(
            first_store
                .verifier
                .verify(first_admitted, &first_store.identity)
                .is_ok()
        );
        let cross_store_admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(Vec::new(), operation())
            .unwrap();

        let error = second_store
            .verifier
            .verify(cross_store_admitted, &second_store.identity)
            .unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::InternalIntegrityFailure);
        assert_eq!(error.reason_code(), "admission-store-identity-mismatch");
    }

    #[test]
    fn evaluation_snapshot_cannot_be_substituted_during_admission() {
        let (issuer, verifier, store_identity) = admission_pair();
        let admitted = issuer
            .record_allow(authorization("work"), snapshot())
            .admit(Vec::new(), operation())
            .unwrap();
        let verified = verifier.verify(admitted, &store_identity).unwrap();

        assert_eq!(
            verified
                .policy_snapshot(&verifier, &store_identity)
                .unwrap(),
            snapshot()
        );
    }

    #[test]
    fn admission_debug_surfaces_redact_protected_fields() {
        const ZONE_SENTINEL: &str = "admission-zone-sentinel";
        const NAME_SENTINEL: &str = "admission-name-sentinel";
        const REF_SENTINEL: &str = "admission-ref-sentinel";
        const UID_SENTINEL: &str = "44444444-4444-4444-8444-444444444444";
        const PAYLOAD_SENTINEL: &str = "admission-payload-sentinel";

        let protected_authorization = AdmittedAuthorization {
            zone: ZoneId::parse(ZONE_SENTINEL).unwrap(),
            subject_ref: ResourceRef::parse(&format!("Provider/{REF_SENTINEL}")).unwrap(),
            subject_uid: ResourceUid::parse(UID_SENTINEL).unwrap(),
            targets: vec![d2b_resource_store::AdmittedAuthorizationTarget {
                resource_type: ResourceTypeName::parse("Host").unwrap(),
                resource_name: Some(ResourceName::parse(NAME_SENTINEL).unwrap()),
                verb: d2b_resource_store::AdmittedVerb::Delete,
                subresource: Some(PAYLOAD_SENTINEL.to_owned()),
                execution_ref: Some(
                    ResourceRef::parse(&format!("Process/{REF_SENTINEL}")).unwrap(),
                ),
            }],
        };
        let (issuer, verifier, store_identity) = admission_pair();
        let issuer_debug = format!("{issuer:?}");
        let verifier_debug = format!("{verifier:?}");
        let store_identity_debug = format!("{store_identity:?}");
        let permit = issuer.record_allow(protected_authorization, snapshot());
        let permit_debug = format!("{permit:?}");
        let mut protected_mutation = mutation(ZONE_SENTINEL);
        protected_mutation.kind = d2b_resource_store::ResourceMutationKind::Delete;
        protected_mutation.target = ResourceRef::parse(&format!("Host/{REF_SENTINEL}")).unwrap();
        protected_mutation.expected =
            d2b_resource_store::ExpectedRevision::Exact(ZoneRevision::new(1));
        protected_mutation.expected_uid = Some(ResourceUid::parse(UID_SENTINEL).unwrap());
        protected_mutation.owner =
            Some(ResourceRef::parse(&format!("Process/{REF_SENTINEL}")).unwrap());
        let admitted = permit
            .admit(
                vec![protected_mutation],
                StoreOperationContext {
                    operation_id: PAYLOAD_SENTINEL.to_owned(),
                    idempotency_key: Some(PAYLOAD_SENTINEL.to_owned()),
                    correlation_id: PAYLOAD_SENTINEL.to_owned(),
                    trace_id: Some(PAYLOAD_SENTINEL.to_owned()),
                    deadline_ms: 1,
                },
            )
            .unwrap();
        let admitted_debug = format!("{admitted:?}");
        let verified = verifier.verify(admitted, &store_identity).unwrap();
        let prepared_debug = format!(
            "{:?}",
            verified.mutations(&verifier, &store_identity).unwrap()[0]
        );
        let verified_debug = format!("{verified:?}");

        for rendered in [
            issuer_debug,
            verifier_debug,
            store_identity_debug,
            permit_debug,
            admitted_debug,
            prepared_debug,
            verified_debug,
        ] {
            for sentinel in [
                ZONE_SENTINEL,
                NAME_SENTINEL,
                REF_SENTINEL,
                UID_SENTINEL,
                PAYLOAD_SENTINEL,
            ] {
                assert!(!rendered.contains(sentinel), "{rendered}");
            }
        }
    }
}
