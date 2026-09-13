//! Zone-global authority operation ledger.
//!
//! The pre-v3 Zone store owned these rows durably. With the store deleted
//! (spec §26, R29) the ledger is process-local: it keeps the admission
//! barrier's operation lifecycle inside one daemon lifetime, and recovery
//! across a restart is the drivers' probe/adopt path rather than a replayed
//! controller checkpoint (R11, R16).
//!
//! Core owns the typed rows and recovery validation; this adapter owns the
//! bytes and the capability binding, exactly as the deleted redb owner did,
//! minus the durable substrate. The provenance the deleted owner read from
//! the store - the authoritative owner row and the live external-NIC
//! inventory - is supplied back through [`AuthorityOwnerProvenance`] and
//! [`ExternalNicRecoveryInventory`]; a claim that cannot be re-proven against
//! the authoritative rows is refused rather than assumed.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_core_controller::authority::{
    AuthorityOperationState, AuthorityStorageClaim, AuthorityStorageOperation,
    ExternalNicRecoveryInventory, claim_digest,
};
use d2b_core_controller::authority_persistence::{
    AuthorityFuture, AuthorityOperationCapability, AuthorityPersistence, AuthorityPersistenceError,
    AuthorityRecoveryData, AuthorityRecoveryProvenance, PreparedAuthorityOperation,
};

/// Trusted owner re-proof for one authority claim, supplied by the daemon that
/// owns the authoritative rows.
///
/// The Zone store that answered this before U14 is gone; the manager plane is
/// the only authority now. Digest equality alone is never sufficient: the
/// claim's owner ref must resolve to the authoritative row's exact uid and
/// generation before the claim is written or rehydrated. A ledger with no
/// resolver refuses every claim.
pub trait AuthorityOwnerProvenance: Send + Sync {
    /// Resolve the `(uid, generation)` the authoritative rows currently hold
    /// for one owner ref. `Ok(None)` means no authoritative row holds the
    /// ref; a read failure is an error, never absence.
    fn owner_identity<'a>(
        &'a self,
        owner_ref: &'a ResourceRef,
    ) -> AuthorityFuture<'a, Option<(ResourceUid, ResourceGeneration)>>;
}

/// One ledger row: the typed operation plus the payload and state the
/// admission barrier reads back by operation id.
pub struct LedgerRow {
    pub operation_id: String,
    pub state: AuthorityOperationState,
    pub claim_digest: String,
    pub store_binding_digest: String,
    pub payload: Vec<u8>,
}

impl core::fmt::Debug for LedgerRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LedgerRow")
            .field("operation_id", &self.operation_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

type Rows = Arc<Mutex<BTreeMap<String, LedgerRow>>>;

/// Process-local authority operation owner for one Zone.
pub struct ZoneAuthorityLedger {
    binding_digest: String,
    rows: Rows,
    owner_provenance: Mutex<Option<Arc<dyn AuthorityOwnerProvenance>>>,
    external_inventory: Mutex<Option<Arc<dyn ExternalNicRecoveryInventory>>>,
    /// The nonce handed to each prepared operation. It is derived per prepare
    /// (never a constant): the capability binding proves a live prepared
    /// operation, so every prepare must carry its own distinct non-zero value.
    prepare_nonce: AtomicU64,
}

impl core::fmt::Debug for ZoneAuthorityLedger {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneAuthorityLedger(<process-local>)")
    }
}

impl ZoneAuthorityLedger {
    /// Bind one ledger to its Zone's admission barrier.
    pub fn new(zone: &ZoneId) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"d2b-zone-authority-ledger/v1\x00");
        digest.update(zone.as_str().as_bytes());
        Self {
            binding_digest: format!("sha256:{:x}", digest.finalize()),
            rows: Arc::new(Mutex::new(BTreeMap::new())),
            owner_provenance: Mutex::new(None),
            external_inventory: Mutex::new(None),
            prepare_nonce: AtomicU64::new(1),
        }
    }

    /// The nonce for one prepared operation: distinct per prepare within this
    /// ledger's lifetime, and never zero (the type refuses zero).
    fn next_prepare_nonce(&self) -> u64 {
        self.prepare_nonce.fetch_add(1, Ordering::Relaxed)
    }

    /// Install the daemon's authoritative owner source. Until one is
    /// installed every claim is refused: a claim that cannot be re-proven is
    /// refused rather than assumed.
    pub fn install_owner_provenance(&self, provenance: Arc<dyn AuthorityOwnerProvenance>) {
        let mut slot = self.owner_provenance.lock().expect("authority ledger lock");
        *slot = Some(provenance);
    }

    /// Install the trusted live external-NIC inventory. Until one is
    /// installed every `ExternalNic` claim is refused.
    pub fn install_external_inventory(&self, inventory: Arc<dyn ExternalNicRecoveryInventory>) {
        let mut slot = self
            .external_inventory
            .lock()
            .expect("authority ledger lock");
        *slot = Some(inventory);
    }

    /// Stable binding digest for one claim: identical inputs produce the same
    /// digest for this ledger's lifetime, so a resumed operation keeps its
    /// fence.
    pub fn authority_binding_digest(&self, claim_digest: &str) -> String {
        let mut digest = Sha256::new();
        digest.update(b"d2b-zone-authority-binding/v1\x00");
        digest.update(self.binding_digest.as_bytes());
        digest.update([0]);
        digest.update(claim_digest.as_bytes());
        format!("sha256:{:x}", digest.finalize())
    }

    /// Every operation the ledger currently holds, ordered by operation id.
    pub fn authority_operations(&self) -> Vec<LedgerRow> {
        let rows = self.rows.lock().expect("authority ledger lock");
        rows.values()
            .map(|row| LedgerRow {
                operation_id: row.operation_id.clone(),
                state: row.state,
                claim_digest: row.claim_digest.clone(),
                store_binding_digest: row.store_binding_digest.clone(),
                payload: row.payload.clone(),
            })
            .collect()
    }

    /// Prepare one operation, idempotently: an already prepared id with the
    /// same claim keeps its capability, and a differing claim is refused as a
    /// conflict rather than silently replacing a live row.
    pub fn prepare_authority_operation(
        &self,
        operation_id: String,
        payload: Vec<u8>,
        claim_digest: &str,
    ) -> Result<ZoneAuthorityCapability, AuthorityPersistenceError> {
        let store_binding_digest = self.authority_binding_digest(claim_digest);
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        match rows.get(&operation_id) {
            Some(row) if row.claim_digest != claim_digest => {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
            Some(_) => {}
            None => {
                rows.insert(
                    operation_id.clone(),
                    LedgerRow {
                        operation_id: operation_id.clone(),
                        state: AuthorityOperationState::Pending,
                        claim_digest: claim_digest.to_owned(),
                        store_binding_digest: store_binding_digest.clone(),
                        payload,
                    },
                );
            }
        }
        Ok(ZoneAuthorityCapability {
            rows: Arc::clone(&self.rows),
            operation_id,
            store_binding_digest,
        })
    }

    /// Resume a prepared non-terminal operation.
    pub fn resume_authority_operation(
        &self,
        operation_id: String,
        binding_digest: &str,
    ) -> Result<ZoneAuthorityCapability, AuthorityPersistenceError> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get(&operation_id)
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != binding_digest {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        Ok(ZoneAuthorityCapability {
            rows: Arc::clone(&self.rows),
            operation_id,
            store_binding_digest: binding_digest.to_owned(),
        })
    }

    /// Re-prove one claim's owner (and, for `ExternalNic` claims, its live
    /// inventory identity) before the ledger touches the row. This is the
    /// fence the deleted durable owner applied against its store.
    async fn prove_claim(
        &self,
        claim: &AuthorityStorageClaim,
    ) -> Result<(), AuthorityPersistenceError> {
        let owner_proof = match claim {
            AuthorityStorageClaim::Generic(claim) => claim.owner_proof(),
            AuthorityStorageClaim::ExternalNic(claim) => claim.owner_proof(),
        };
        let Some(owner_ref) = owner_proof.resource_ref() else {
            return Err(AuthorityPersistenceError::RowInvalid);
        };
        let provenance = self
            .owner_provenance
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?
            .clone()
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        match provenance.owner_identity(owner_ref).await? {
            Some((uid, generation))
                if uid == *owner_proof.resource_uid()
                    && generation == owner_proof.generation() => {}
            _ => return Err(AuthorityPersistenceError::RowInvalid),
        }
        if let AuthorityStorageClaim::ExternalNic(claim) = claim {
            let inventory = self
                .external_inventory
                .lock()
                .map_err(|_| AuthorityPersistenceError::StateInvalid)?
                .clone()
                .ok_or(AuthorityPersistenceError::RowInvalid)?;
            if !inventory.contains_identity(claim.host_uid(), claim.identity_digest()) {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
        }
        Ok(())
    }

    fn record(
        &self,
        capability: &AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> Result<(), AuthorityPersistenceError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get_mut(capability.operation_id())
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != capability.store_binding_digest() {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        apply_transition(row, state)
    }

    fn record_effect(
        &self,
        capability: &AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> Result<(), AuthorityPersistenceError> {
        if is_terminal(state) {
            return Err(AuthorityPersistenceError::StateInvalid);
        }
        self.record(capability, state)
    }
}

/// Whether one state is retired for good: no effect may write it, and no
/// effect may follow it.
fn is_terminal(state: AuthorityOperationState) -> bool {
    matches!(
        state,
        AuthorityOperationState::Closed | AuthorityOperationState::Released
    )
}

/// The ledger lifecycle, exactly the table the deleted Zone store enforced:
/// effect outcomes move between effect states, closure hands a row to
/// `Closed`, and a terminal row never reopens.
fn state_transition_allowed(
    current: AuthorityOperationState,
    next: AuthorityOperationState,
) -> bool {
    use AuthorityOperationState::{
        Closed, Closing, EffectConfirmed, EffectRetryable, EffectTerminal, Pending, Released,
    };
    if current == next {
        return true;
    }
    match current {
        Pending | EffectConfirmed | EffectRetryable | EffectTerminal => {
            matches!(
                next,
                EffectConfirmed | EffectRetryable | EffectTerminal | Closing | Closed
            )
        }
        Closing => matches!(next, Closed | Released),
        Closed => next == Released,
        Released => false,
    }
}

fn apply_transition(
    row: &mut LedgerRow,
    state: AuthorityOperationState,
) -> Result<(), AuthorityPersistenceError> {
    if !state_transition_allowed(row.state, state) {
        return Err(AuthorityPersistenceError::StateInvalid);
    }
    row.state = state;
    Ok(())
}

/// Capability bound to one ledger row: the process-local fence that permits a
/// state transition on exactly this operation.
pub struct ZoneAuthorityCapability {
    rows: Rows,
    operation_id: String,
    store_binding_digest: String,
}

impl core::fmt::Debug for ZoneAuthorityCapability {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneAuthorityCapability(<ledger-bound>)")
    }
}

impl ZoneAuthorityCapability {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub async fn record_effect(
        &self,
        state: AuthorityOperationState,
    ) -> Result<(), AuthorityPersistenceError> {
        if is_terminal(state) {
            return Err(AuthorityPersistenceError::StateInvalid);
        }
        self.record(state)
    }

    pub async fn record_close(&self) -> Result<(), AuthorityPersistenceError> {
        self.record(AuthorityOperationState::Closed)
    }

    pub async fn release(&self) -> Result<(), AuthorityPersistenceError> {
        self.record(AuthorityOperationState::Released)
    }

    fn record(&self, state: AuthorityOperationState) -> Result<(), AuthorityPersistenceError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get_mut(&self.operation_id)
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != self.store_binding_digest {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        apply_transition(row, state)
    }
}

impl AuthorityPersistence for ZoneAuthorityLedger {
    fn prepare<'a>(
        &'a self,
        operation_id: &'a str,
        claim: &'a AuthorityStorageClaim,
    ) -> AuthorityFuture<'a, PreparedAuthorityOperation> {
        Box::pin(async move {
            let claim_digest =
                claim_digest(claim).map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            let store_binding_digest = self.authority_binding_digest(&claim_digest);
            let row = AuthorityStorageOperation {
                operation_id: operation_id.to_owned(),
                claim: claim.clone(),
                state: AuthorityOperationState::Pending,
                claim_digest: claim_digest.clone(),
                store_binding_digest: store_binding_digest.clone(),
            };
            // The old durable fence lived here: prove the owner against the
            // authoritative rows before a pending row exists.
            self.validate(&row).await?;
            let payload =
                serde_json::to_vec(&row).map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            // One atomic check-and-insert: a concurrent prepare of the same id
            // keeps the identical row or is refused as a claim conflict -
            // never a silent replacement of a live row.
            self.prepare_authority_operation(operation_id.to_owned(), payload, &claim_digest)?;
            let nonce = self.next_prepare_nonce();
            PreparedAuthorityOperation::new(operation_id.to_owned(), store_binding_digest, nonce)
        })
    }

    fn record_effect<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record_effect(capability, state) })
    }

    fn record_close<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record(capability, AuthorityOperationState::Closed) })
    }

    fn release<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record(capability, AuthorityOperationState::Released) })
    }

    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData> {
        // Nothing survives the process: a restart re-derives authority state
        // through driver recovery rather than replaying checkpoints.
        Box::pin(async move { Ok(AuthorityRecoveryData::new(Vec::new(), BTreeMap::new())) })
    }
}

impl AuthorityRecoveryProvenance for ZoneAuthorityLedger {
    fn validate<'a>(&'a self, operation: &'a AuthorityStorageOperation) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let claim_digest = claim_digest(&operation.claim)
                .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            if claim_digest != operation.claim_digest
                || self.authority_binding_digest(&claim_digest) != operation.store_binding_digest
            {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
            if is_terminal(operation.state) {
                // A retired row holds no active authority to re-prove.
                return Ok(());
            }
            self.prove_claim(&operation.claim).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_core_controller::authority::{
        AuthorityCloseOutcome, AuthorityEffectOutcome, AuthorityRequest, AuthorityReservation,
        HostGlobalAuthorityIndex,
    };

    const OWNER_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const HOST_UID: &str = "123e4567-e89b-42d3-a456-4266141740aa";
    const ZONE_UID: &str = "123e4567-e89b-42d3-a456-4266141740bb";
    const OWNER_REF: &str = "Device/gpu-a";

    fn zone() -> ZoneId {
        ZoneId::parse("work").unwrap()
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn generation(value: u64) -> ResourceGeneration {
        ResourceGeneration::new(value).unwrap()
    }

    /// Trusted resolver fixture: the authoritative rows one ledger sees.
    #[derive(Default)]
    struct FixedOwnerProvenance {
        rows: BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>,
    }

    impl AuthorityOwnerProvenance for FixedOwnerProvenance {
        fn owner_identity<'a>(
            &'a self,
            owner_ref: &'a ResourceRef,
        ) -> AuthorityFuture<'a, Option<(ResourceUid, ResourceGeneration)>> {
            let identity = self.rows.get(owner_ref).cloned();
            Box::pin(async move { Ok(identity) })
        }
    }

    /// Trusted live external-NIC inventory fixture.
    #[derive(Default)]
    struct FixedExternalNicInventory {
        identities: Vec<(ResourceUid, String)>,
    }

    impl ExternalNicRecoveryInventory for FixedExternalNicInventory {
        fn contains_identity(&self, host_uid: &ResourceUid, identity_digest: &str) -> bool {
            self.identities
                .iter()
                .any(|(host, digest)| host == host_uid && digest == identity_digest)
        }
    }

    fn ledger_with(owner_ref: &str, owner_uid: &str, owner_generation: u64) -> ZoneAuthorityLedger {
        let ledger = ZoneAuthorityLedger::new(&zone());
        let mut provenance = FixedOwnerProvenance::default();
        provenance.rows.insert(
            ResourceRef::parse(owner_ref).unwrap(),
            (uid(owner_uid), generation(owner_generation)),
        );
        ledger.install_owner_provenance(Arc::new(provenance));
        ledger
    }

    fn generic_claim(owner_ref: &str, owner_uid: &str, owner_generation: u64) -> AuthorityStorageClaim {
        let request = AuthorityRequest::gpu_from_core(
            uid(HOST_UID),
            ResourceRef::parse(owner_ref).unwrap(),
            uid(owner_uid),
            generation(owner_generation),
            [0x11; 32],
            false,
            1,
        )
        .expect("a GPU authority claim");
        AuthorityStorageClaim::Generic(request.durable_claim())
    }

    fn external_nic_claim(owner_generation: u64, identity_digest: &str) -> AuthorityStorageClaim {
        // `ExternalNicClaimRequest`'s constructors are Core-private, so the
        // stored row is built through the same wire shape the deleted store
        // persisted.
        serde_json::from_value(serde_json::json!({
            "externalNic": {
                "hostUid": HOST_UID,
                "identityDigest": identity_digest,
                "zoneUid": ZONE_UID,
                "macvtapMode": "bridge",
                "sharingPolicy": "exclusive",
                "signedMaxHolders": 1,
                "ownerProof": {
                    "resourceRef": OWNER_REF,
                    "resourceUid": OWNER_UID,
                    "generation": owner_generation,
                },
            }
        }))
        .expect("a stored external NIC claim")
    }

    #[tokio::test]
    async fn prepare_refuses_a_claim_without_a_trusted_owner_source() {
        let claim = generic_claim(OWNER_REF, OWNER_UID, 1);
        let bare = ZoneAuthorityLedger::new(&zone());
        assert!(matches!(
            bare.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        assert!(bare.authority_operations().is_empty());
    }

    #[tokio::test]
    async fn prepare_refuses_a_claim_the_authoritative_rows_do_not_hold() {
        let claim = generic_claim(OWNER_REF, OWNER_UID, 2);
        // No authoritative row holds the owner ref.
        let empty = ledger_with("Device/gpu-other", OWNER_UID, 2);
        assert!(matches!(
            empty.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        // The row moved on, so the claim's generation is stale.
        let stale = ledger_with(OWNER_REF, OWNER_UID, 1);
        assert!(matches!(
            stale.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        // The row was replaced under the same ref, so the uid no longer matches.
        let replaced = ledger_with(OWNER_REF, HOST_UID, 2);
        assert!(matches!(
            replaced.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        assert!(stale.authority_operations().is_empty());
    }

    #[tokio::test]
    async fn prepare_admits_a_claim_the_authoritative_rows_hold() {
        let ledger = ledger_with(OWNER_REF, OWNER_UID, 2);
        let claim = generic_claim(OWNER_REF, OWNER_UID, 2);
        assert!(ledger.prepare("operation-a", &claim).await.is_ok());
        assert!(ledger.prepare("operation-a", &claim).await.is_ok());
        let rows = ledger.authority_operations();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].operation_id, "operation-a");
        assert_eq!(rows[0].state, AuthorityOperationState::Pending);
    }

    #[tokio::test]
    async fn every_prepared_operation_carries_its_own_nonce() {
        let ledger = ledger_with(OWNER_REF, OWNER_UID, 2);
        let claim = generic_claim(OWNER_REF, OWNER_UID, 2);
        let first = ledger.prepare("operation-a", &claim).await.expect("first prepare");
        let second = ledger.prepare("operation-b", &claim).await.expect("second prepare");
        // The nonce is the capability's live binding, not a constant: each
        // prepare mints its own non-zero value, including a repeat prepare of
        // the same id (which returns a fresh binding to the same live row).
        assert_ne!(first.nonce(), 0);
        assert_ne!(second.nonce(), 0);
        assert_ne!(
            first.nonce(),
            second.nonce(),
            "two prepares must not share one nonce"
        );
        let repeat = ledger.prepare("operation-a", &claim).await.expect("repeat prepare");
        assert_ne!(repeat.nonce(), first.nonce());
        assert_eq!(ledger.authority_operations().len(), 2);
    }

    #[tokio::test]
    async fn prepare_refuses_a_second_claim_for_a_live_operation() {
        let ledger = ledger_with(OWNER_REF, OWNER_UID, 2);
        let first = generic_claim(OWNER_REF, OWNER_UID, 2);
        assert!(ledger.prepare("operation-a", &first).await.is_ok());
        // The same id with a different claim must not replace the live row or
        // hand out a capability bound to a row that does not exist.
        assert!(matches!(
            ledger
                .prepare("operation-a", &generic_claim("Device/gpu-b", HOST_UID, 1))
                .await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        let rows = ledger.authority_operations();
        assert_eq!(rows.len(), 1);
        // The live row still carries the first claim: the refused second
        // prepare neither replaced it nor rebound its digest.
        assert_eq!(rows[0].claim_digest, claim_digest(&first).expect("claim digest"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_prepares_of_one_operation_admit_exactly_one_claim() {
        let ledger = Arc::new(ledger_with(OWNER_REF, OWNER_UID, 2));
        let persistence = Arc::clone(&ledger) as Arc<dyn AuthorityPersistence>;
        let first = generic_claim(OWNER_REF, OWNER_UID, 2);
        let second = generic_claim("Device/gpu-b", HOST_UID, 1);
        let (first, second) = tokio::join!(
            persistence.prepare("operation-a", &first),
            persistence.prepare("operation-a", &second),
        );
        assert!(
            first.is_ok() ^ second.is_ok(),
            "exactly one concurrent prepare may claim a live operation",
        );
        assert_eq!(ledger.authority_operations().len(), 1);
    }

    #[tokio::test]
    async fn external_nic_claims_require_the_live_inventory_identity() {
        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let claim = external_nic_claim(1, digest);
        let ledger = ledger_with(OWNER_REF, OWNER_UID, 1);
        // Owner re-proven, but no trusted inventory installed yet.
        assert!(matches!(
            ledger.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        // The inventory does not contain this claim's identity.
        let mut inventory = FixedExternalNicInventory::default();
        inventory
            .identities
            .push((uid(HOST_UID), "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()));
        ledger.install_external_inventory(Arc::new(inventory));
        assert!(matches!(
            ledger.prepare("operation-a", &claim).await,
            Err(AuthorityPersistenceError::RowInvalid)
        ));
        // The live inventory holds exactly this identity.
        let mut inventory = FixedExternalNicInventory::default();
        inventory
            .identities
            .push((uid(HOST_UID), digest.to_owned()));
        ledger.install_external_inventory(Arc::new(inventory));
        assert!(ledger.prepare("operation-a", &claim).await.is_ok());
    }

    #[tokio::test]
    async fn a_retained_capability_cannot_reopen_a_terminal_row() {
        let ledger = ledger_with(OWNER_REF, OWNER_UID, 2);
        let capability = ledger
            .prepare_authority_operation(
                "operation-a".to_owned(),
                b"payload".to_vec(),
                "claim-digest",
            )
            .unwrap();
        capability
            .record_effect(AuthorityOperationState::EffectConfirmed)
            .await
            .unwrap();
        // An effect state is never a closure state.
        assert!(matches!(
            capability.record_effect(AuthorityOperationState::Closed).await,
            Err(AuthorityPersistenceError::StateInvalid)
        ));
        capability.record_close().await.unwrap();
        // The capability outlives the effect: it must not re-open the row.
        assert!(matches!(
            capability
                .record_effect(AuthorityOperationState::EffectConfirmed)
                .await,
            Err(AuthorityPersistenceError::StateInvalid)
        ));
        assert_eq!(
            ledger.authority_operations()[0].state,
            AuthorityOperationState::Closed
        );
        capability.release().await.unwrap();
        assert!(matches!(
            capability
                .record_effect(AuthorityOperationState::EffectRetryable)
                .await,
            Err(AuthorityPersistenceError::StateInvalid)
        ));
        assert!(matches!(
            capability.record_close().await,
            Err(AuthorityPersistenceError::StateInvalid)
        ));
        assert_eq!(
            ledger.authority_operations()[0].state,
            AuthorityOperationState::Released
        );
    }

    #[tokio::test]
    async fn the_core_reservation_lifecycle_retires_the_ledger_row() {
        let ledger = Arc::new(ledger_with(OWNER_REF, OWNER_UID, 2));
        let index = Arc::new(tokio::sync::Mutex::new(
            HostGlobalAuthorityIndex::new_for_tests_ready(),
        ));
        let request = AuthorityRequest::gpu_from_core(
            uid(HOST_UID),
            ResourceRef::parse(OWNER_REF).unwrap(),
            uid(OWNER_UID),
            generation(2),
            [0x11; 32],
            false,
            1,
        )
        .unwrap();
        let mut reservation = AuthorityReservation::reserve_durable(
            Arc::clone(&index),
            Arc::clone(&ledger) as Arc<dyn AuthorityPersistence>,
            "operation-a",
            request,
        )
        .await
        .expect("a proven reservation");
        assert_eq!(
            reservation
                .dispatch(|_lease| async {
                    Ok::<AuthorityEffectOutcome, ()>(AuthorityEffectOutcome::Confirmed)
                })
                .await
                .unwrap(),
            AuthorityEffectOutcome::Confirmed
        );
        assert_eq!(
            ledger.authority_operations()[0].state,
            AuthorityOperationState::EffectConfirmed
        );
        reservation
            .close_then_release(|| AuthorityCloseOutcome::Confirmed)
            .await
            .unwrap();
        assert_eq!(
            ledger.authority_operations()[0].state,
            AuthorityOperationState::Released
        );
    }
}
