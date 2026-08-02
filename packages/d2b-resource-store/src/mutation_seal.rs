//! Store-owned capability for one verified mutation commit.

use std::sync::Arc;

use d2b_contracts::v3::{ResourceUid, RetryClass, ZoneId};

use crate::{
    AdmittedAuthorization, PolicySnapshot, PreparedStoreMutation, SealIdentityMismatch, StoreError,
    StoreErrorKind, StoreOperationContext, StoreSlot,
};

mod authority {
    pub struct SealAuthority;
}

/// Declared identity of one provisioned store and its operator-facing slot.
#[derive(Clone)]
pub struct StoreSealIdentity {
    slot: StoreSlot,
    zone: ZoneId,
    store_uuid: ResourceUid,
}

impl StoreSealIdentity {
    pub fn new(slot: StoreSlot, zone: ZoneId, store_uuid: ResourceUid) -> Self {
        Self {
            slot,
            zone,
            store_uuid,
        }
    }

    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub const fn slot(&self) -> StoreSlot {
        self.slot
    }
}

/// Payload prepared after admission and before the store-owned seal.
pub struct MutationSealBody {
    pub mutations: Vec<PreparedStoreMutation>,
    pub authorization: AdmittedAuthorization,
    pub policy_snapshot: PolicySnapshot,
    pub operation: StoreOperationContext,
}

/// Evidence that carries one private authority, store identity, and payload.
pub struct SealedMutation {
    authority: Arc<authority::SealAuthority>,
    store: StoreSealIdentity,
    body: MutationSealBody,
}

/// Mutation evidence after the paired acceptor has opened it.
pub struct OpenedMutation {
    body: MutationSealBody,
}

impl OpenedMutation {
    pub fn body(&self) -> &MutationSealBody {
        &self.body
    }

    pub fn into_body(self) -> MutationSealBody {
        self.body
    }
}

/// The only issuer for a store-owned mutation seal.
pub struct MutationSealIssuer {
    authority: Arc<authority::SealAuthority>,
    store: StoreSealIdentity,
}

/// The paired acceptor for a store-owned mutation seal.
pub struct MutationSealAcceptor {
    authority: Arc<authority::SealAuthority>,
    store: StoreSealIdentity,
}

/// Create the paired issuer and acceptor for one declared store.
pub fn mutation_seal_pair(store: StoreSealIdentity) -> (MutationSealIssuer, MutationSealAcceptor) {
    let authority = Arc::new(authority::SealAuthority);
    (
        MutationSealIssuer {
            authority: Arc::clone(&authority),
            store: store.clone(),
        },
        MutationSealAcceptor { authority, store },
    )
}

impl MutationSealIssuer {
    /// Consume the payload into evidence bound to this issuer's store.
    pub fn seal(&self, body: MutationSealBody) -> SealedMutation {
        SealedMutation {
            authority: Arc::clone(&self.authority),
            store: self.store.clone(),
            body,
        }
    }
}

impl MutationSealAcceptor {
    /// Diagnose the declared identity against the store being installed.
    pub fn diagnose(&self, store: &StoreSealIdentity) -> Result<(), SealIdentityMismatch> {
        diagnose_identity(&self.store, store)
    }

    pub const fn declared_slot(&self) -> StoreSlot {
        self.store.slot()
    }

    /// Consume evidence and expose its payload only to the paired acceptor.
    pub fn open(&self, sealed: SealedMutation) -> Result<OpenedMutation, StoreError> {
        if !Arc::ptr_eq(&self.authority, &sealed.authority) {
            return Err(self.integrity("mutation-seal-authority-mismatch"));
        }
        if diagnose_identity(&sealed.store, &self.store).is_err() {
            return Err(self.integrity("mutation-seal-store-identity-mismatch"));
        }
        Ok(OpenedMutation { body: sealed.body })
    }

    fn integrity(&self, reason_code: &'static str) -> StoreError {
        StoreError::new(
            StoreErrorKind::InternalIntegrityFailure,
            None,
            None,
            RetryClass::Never,
            reason_code,
        )
        .with_store_slot(self.store.slot())
    }
}

fn assert_mutation_seal_types_have_no_minting_traits() {
    trait CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<A> {
        fn some_item() {}
    }
    impl<T: ?Sized> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<()> for T {}
    impl<T: Clone> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u8> for T {}
    impl<T: Copy> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u16> for T {}
    impl<T: Default> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u32> for T {}
    impl<T: core::fmt::Debug> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u64> for T {}
    impl<T: From<()>> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u128> for T {}
    let _ = <SealedMutation as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
    let _ =
        <MutationSealIssuer as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
    let _ = <MutationSealAcceptor as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<
        _,
    >>::some_item;
    let _ = <OpenedMutation as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
}

const _: fn() = assert_mutation_seal_types_have_no_minting_traits;

fn diagnose_identity(
    declared: &StoreSealIdentity,
    expected: &StoreSealIdentity,
) -> Result<(), SealIdentityMismatch> {
    if declared.zone != expected.zone {
        return Err(SealIdentityMismatch::Zone);
    }
    if declared.store_uuid != expected.store_uuid {
        return Err(SealIdentityMismatch::Store);
    }
    Ok(())
}

#[test]
fn open_rejects_same_authority_with_mismatched_declared_identity() {
    let slot = crate::StoreSlot::new(7).unwrap();
    let zone = d2b_contracts::v3::ZoneId::parse("work").unwrap();
    let (issuer, acceptor) = mutation_seal_pair(StoreSealIdentity::new(
        slot,
        zone.clone(),
        d2b_contracts::v3::ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
    ));
    let mut sealed = issuer.seal(MutationSealBody {
        mutations: Vec::new(),
        authorization: crate::AdmittedAuthorization {
            zone: zone.clone(),
            subject_ref: d2b_contracts::v3::ResourceRef::parse("Provider/system-core").unwrap(),
            subject_uid: d2b_contracts::v3::ResourceUid::parse(
                "33333333-3333-4333-8333-333333333333",
            )
            .unwrap(),
            targets: Vec::new(),
        },
        policy_snapshot: crate::PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: d2b_contracts::v3::ConfigurationGeneration::new(9)
                .unwrap(),
            controller_generation: None,
        },
        operation: crate::StoreOperationContext {
            operation_id: "open".to_owned(),
            idempotency_key: None,
            correlation_id: "open".to_owned(),
            trace_id: None,
            deadline_ms: 1,
        },
    });
    sealed.store = StoreSealIdentity::new(
        slot,
        zone,
        d2b_contracts::v3::ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap(),
    );

    let error = acceptor
        .open(sealed)
        .err()
        .expect("mismatched declared identity must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-store-identity-mismatch");
    assert_eq!(error.store_slot(), Some(slot));
}
