//! Host-global authority admission for external physical NICs.
//!
//! Core resolves an authored interface selector through trusted Host inventory,
//! derives an opaque key, and admits the claim before any macvtap or VMM effect.
//! The key, resolved inventory identity, and owner proof have no serialization or
//! display surface.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    ResourceGeneration, ResourceUid, UpdateState,
    ifname::IfName,
    network::{
        ExternalNicAdmissionError, ExternalNicAuthorityStatus, ExternalNicClaim, MacvtapMode,
        SharingPolicy, admit_external_nic_claims,
    },
    resource_schema::canonical_digest,
};

/// Domain tag for the Core-derived external physical-NIC identity.
pub const EXTERNAL_PHYSICAL_NIC_IDENTITY_DOMAIN: &str = "external-physical-nic/v1";
/// Authority class used in the Host-global index.
pub const EXTERNAL_PHYSICAL_NIC_AUTHORITY_CLASS: &str = "external-physical-nic";
const MAX_RESOLVED_NIC_IDENTITY_BYTES: usize = 256;

/// One stable physical-NIC identity resolved from trusted Host inventory.
///
/// This is not an authored interface selector and cannot be serialized into a
/// resource. Core derives the authority key from these private bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedExternalNicIdentity(Vec<u8>);

impl ResolvedExternalNicIdentity {
    /// Record a stable identity returned by the trusted inventory adapter.
    pub fn from_trusted_inventory(bytes: impl Into<Vec<u8>>) -> Result<Self, AuthorityError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > MAX_RESOLVED_NIC_IDENTITY_BYTES {
            return Err(AuthorityError::InvalidTrustedInventoryIdentity);
        }
        Ok(Self(bytes))
    }
}

impl core::fmt::Debug for ResolvedExternalNicIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResolvedExternalNicIdentity(<redacted>)")
    }
}

/// Trusted Host inventory used to resolve authored interface selectors.
#[derive(Default)]
pub struct TrustedExternalNicInventory {
    entries: BTreeMap<IfName, ResolvedExternalNicIdentity>,
}

impl TrustedExternalNicInventory {
    /// Add one resolver-owned inventory row.
    pub fn insert(
        &mut self,
        selector: IfName,
        identity: ResolvedExternalNicIdentity,
    ) -> Result<(), AuthorityError> {
        if self.entries.insert(selector, identity).is_some() {
            return Err(AuthorityError::DuplicateTrustedInventorySelector);
        }
        Ok(())
    }

    /// Resolve an authored selector without exposing the derived authority key.
    pub fn resolve(
        &self,
        selector: &IfName,
    ) -> Result<ResolvedExternalNicIdentity, AuthorityError> {
        self.entries
            .get(selector)
            .cloned()
            .ok_or(AuthorityError::TrustedInventorySelectorNotFound)
    }
}

impl core::fmt::Debug for TrustedExternalNicInventory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrustedExternalNicInventory")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

/// Exact resource identity used to adopt or release one authority holder.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalNicOwnerProof {
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl ExternalNicOwnerProof {
    /// Bind an owner proof to an exact resource identity and generation.
    pub const fn new(resource_uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self {
            resource_uid,
            generation,
        }
    }
}

impl core::fmt::Debug for ExternalNicOwnerProof {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicOwnerProof(<redacted>)")
    }
}

/// Complete pre-effect request for one external physical-NIC claim.
pub struct ExternalNicClaimRequest {
    host_uid: ResourceUid,
    identity: ResolvedExternalNicIdentity,
    claim: ExternalNicClaim,
    owner_proof: ExternalNicOwnerProof,
    signed_max_holders: usize,
}

impl ExternalNicClaimRequest {
    /// Construct a request from a trusted inventory result and signed quota.
    pub fn new(
        host_uid: ResourceUid,
        identity: ResolvedExternalNicIdentity,
        claim: ExternalNicClaim,
        owner_proof: ExternalNicOwnerProof,
        signed_max_holders: usize,
    ) -> Result<Self, AuthorityError> {
        if signed_max_holders == 0 || signed_max_holders > u32::MAX as usize {
            return Err(AuthorityError::InvalidSignedHolderLimit);
        }
        Ok(Self {
            host_uid,
            identity,
            claim,
            owner_proof,
            signed_max_holders,
        })
    }
}

impl core::fmt::Debug for ExternalNicClaimRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicClaimRequest(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExternalNicAuthorityKey {
    host_uid: ResourceUid,
    opaque_digest: String,
}

impl ExternalNicAuthorityKey {
    fn derive(host_uid: ResourceUid, identity: &ResolvedExternalNicIdentity) -> Self {
        let mut framed = Vec::with_capacity(8 + identity.0.len());
        framed.extend_from_slice(&(identity.0.len() as u64).to_be_bytes());
        framed.extend_from_slice(&identity.0);
        Self {
            host_uid,
            opaque_digest: canonical_digest(EXTERNAL_PHYSICAL_NIC_IDENTITY_DOMAIN, &framed),
        }
    }
}

impl core::fmt::Debug for ExternalNicAuthorityKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicAuthorityKey(<redacted>)")
    }
}

#[derive(Clone)]
struct Holder {
    claim: ExternalNicClaim,
    owner_proof: ExternalNicOwnerProof,
}

struct AuthorityEntry {
    holders: Vec<Holder>,
    signed_max_holders: usize,
}

/// Proof that Core admitted a Host-global claim before an external effect.
///
/// The lease is deliberately non-serializable and does not reveal its key or
/// owner proof.
pub struct ExternalNicLease {
    key: ExternalNicAuthorityKey,
    owner_proof: ExternalNicOwnerProof,
}

impl core::fmt::Debug for ExternalNicLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicLease(<redacted>)")
    }
}

/// Closed effect result retained beside an admitted lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalNicEffectOutcome {
    /// The effect completed and observation confirmed it.
    Confirmed,
    /// The effect may be retried while the authority remains held.
    RetryableFailure,
    /// The effect failed terminally while the authority remains held for drain.
    TerminalFailure,
}

/// Result of gating one host effect on authority admission.
pub struct ExternalNicEffectGate {
    lease: ExternalNicLease,
    outcome: ExternalNicEffectOutcome,
}

impl ExternalNicEffectGate {
    /// Consume the gate into its retained authority lease.
    pub fn into_lease(self) -> ExternalNicLease {
        self.lease
    }

    /// Return the closed effect outcome.
    pub const fn outcome(&self) -> ExternalNicEffectOutcome {
        self.outcome
    }
}

impl core::fmt::Debug for ExternalNicEffectGate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalNicEffectGate")
            .field("lease", &self.lease)
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// Closed result of attempting to close old macvtap and VMM ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalNicCloseOutcome {
    /// Every old holder and FD is confirmed closed.
    Confirmed,
    /// Closure is incomplete, so the authority must remain held.
    RetryableFailure,
}

/// Restart-adoption result for one exact owner proof.
pub enum ExternalNicAdoption {
    /// Exactly one recovered owner matched the indexed claim.
    Adopted(ExternalNicLease),
    /// No matching indexed and observed owner exists.
    Missing,
    /// Recovery found more than one matching owner and effects stay quarantined.
    QuarantinedAmbiguous,
}

impl core::fmt::Debug for ExternalNicAdoption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Adopted(_) => f.write_str("ExternalNicAdoption::Adopted(<redacted>)"),
            Self::Missing => f.write_str("ExternalNicAdoption::Missing"),
            Self::QuarantinedAmbiguous => f.write_str("ExternalNicAdoption::QuarantinedAmbiguous"),
        }
    }
}

/// Closed, identity-free authority failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    /// Trusted inventory returned an absent or oversized stable identity.
    InvalidTrustedInventoryIdentity,
    /// The trusted inventory contains the same selector twice.
    DuplicateTrustedInventorySelector,
    /// The authored selector did not resolve in trusted inventory.
    TrustedInventorySelectorNotFound,
    /// The signed quota is zero or cannot be represented in bounded status.
    InvalidSignedHolderLimit,
    /// Claim compatibility or isolation admission failed.
    Admission(ExternalNicAdmissionError),
    /// A lease no longer names an indexed claim.
    UnknownClaim,
    /// A lease does not match the indexed owner proof.
    OwnerProofMismatch,
    /// Macvtap or VMM ownership was not confirmed closed.
    AttachmentCloseUnconfirmed,
}

impl AuthorityError {
    /// Return the stable, identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidTrustedInventoryIdentity => "invalid-trusted-inventory-identity",
            Self::DuplicateTrustedInventorySelector => "duplicate-trusted-inventory-selector",
            Self::TrustedInventorySelectorNotFound => "trusted-inventory-selector-not-found",
            Self::InvalidSignedHolderLimit => "invalid-signed-holder-limit",
            Self::Admission(reason) => reason.code(),
            Self::UnknownClaim => "external-physical-nic-claim-missing",
            Self::OwnerProofMismatch => "external-physical-nic-owner-proof-mismatch",
            Self::AttachmentCloseUnconfirmed => "external-physical-nic-close-unconfirmed",
        }
    }
}

impl core::fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AuthorityError {}

impl From<ExternalNicAdmissionError> for AuthorityError {
    fn from(value: ExternalNicAdmissionError) -> Self {
        Self::Admission(value)
    }
}

/// Core-owned Host-global external physical-NIC authority index.
#[derive(Default)]
pub struct HostGlobalAuthorityIndex {
    external_nics: BTreeMap<ExternalNicAuthorityKey, AuthorityEntry>,
}

impl HostGlobalAuthorityIndex {
    /// Admit the claim, then and only then invoke one host effect.
    pub fn admit_before_effect(
        &mut self,
        request: ExternalNicClaimRequest,
        effect: impl FnOnce(&ExternalNicLease) -> ExternalNicEffectOutcome,
    ) -> Result<ExternalNicEffectGate, AuthorityError> {
        let lease = self.admit(request)?;
        let outcome = effect(&lease);
        Ok(ExternalNicEffectGate { lease, outcome })
    }

    /// Return the bounded public observation for one resolved authority.
    pub fn external_nic_status(
        &self,
        host_uid: ResourceUid,
        identity: &ResolvedExternalNicIdentity,
    ) -> Option<ExternalNicAuthorityStatus> {
        let key = ExternalNicAuthorityKey::derive(host_uid, identity);
        let entry = self.external_nics.get(&key)?;
        let all_multiplexable = entry.holders.iter().all(|holder| {
            holder.claim.macvtap_mode() == MacvtapMode::Bridge
                && holder.claim.sharing_policy() == SharingPolicy::Multiplexed
        });
        let arbitration = if all_multiplexable {
            SharingPolicy::Multiplexed
        } else {
            SharingPolicy::Exclusive
        };
        Some(ExternalNicAuthorityStatus::new(
            all_multiplexable && entry.holders.len() < entry.signed_max_holders,
            entry.holders.len() as u32,
            0,
            arbitration,
            UpdateState::Current,
        ))
    }

    /// Adopt only one exact recovered owner; duplicate observations quarantine.
    pub fn adopt(
        &self,
        host_uid: ResourceUid,
        identity: &ResolvedExternalNicIdentity,
        owner_proof: &ExternalNicOwnerProof,
        recovered_owner_proofs: &[ExternalNicOwnerProof],
    ) -> ExternalNicAdoption {
        let key = ExternalNicAuthorityKey::derive(host_uid, identity);
        let Some(entry) = self.external_nics.get(&key) else {
            return ExternalNicAdoption::Missing;
        };
        if recovered_owner_proofs
            .iter()
            .filter(|proof| *proof == owner_proof)
            .count()
            > 1
        {
            return ExternalNicAdoption::QuarantinedAmbiguous;
        }
        let observed = recovered_owner_proofs
            .iter()
            .filter(|proof| *proof == owner_proof)
            .count()
            == 1;
        let indexed = entry
            .holders
            .iter()
            .any(|holder| &holder.owner_proof == owner_proof);
        if observed && indexed {
            ExternalNicAdoption::Adopted(ExternalNicLease {
                key,
                owner_proof: owner_proof.clone(),
            })
        } else {
            ExternalNicAdoption::Missing
        }
    }

    /// Close the old attachment before releasing its authority claim.
    pub fn close_then_release(
        &mut self,
        lease: &ExternalNicLease,
        close: impl FnOnce() -> ExternalNicCloseOutcome,
    ) -> Result<(), AuthorityError> {
        if close() != ExternalNicCloseOutcome::Confirmed {
            return Err(AuthorityError::AttachmentCloseUnconfirmed);
        }
        self.release(lease)
    }

    /// Drain and release an old claim before admitting a disruptive replacement.
    pub fn replace_after_close(
        &mut self,
        lease: &ExternalNicLease,
        replacement: ExternalNicClaimRequest,
        close: impl FnOnce() -> ExternalNicCloseOutcome,
    ) -> Result<ExternalNicLease, AuthorityError> {
        self.close_then_release(lease, close)?;
        self.admit(replacement)
    }

    fn admit(
        &mut self,
        request: ExternalNicClaimRequest,
    ) -> Result<ExternalNicLease, AuthorityError> {
        let key = ExternalNicAuthorityKey::derive(request.host_uid, &request.identity);
        if let Some(entry) = self.external_nics.get_mut(&key) {
            if let Some(holder) = entry
                .holders
                .iter()
                .find(|holder| holder.owner_proof == request.owner_proof)
            {
                if holder.claim == request.claim {
                    return Ok(ExternalNicLease {
                        key,
                        owner_proof: request.owner_proof,
                    });
                }
                return Err(AuthorityError::OwnerProofMismatch);
            }
            let signed_limit = entry.signed_max_holders.min(request.signed_max_holders);
            let mut claims: Vec<ExternalNicClaim> = entry
                .holders
                .iter()
                .map(|holder| holder.claim.clone())
                .collect();
            claims.push(request.claim.clone());
            admit_external_nic_claims(&claims, signed_limit)?;
            entry.signed_max_holders = signed_limit;
            entry.holders.push(Holder {
                claim: request.claim,
                owner_proof: request.owner_proof.clone(),
            });
        } else {
            admit_external_nic_claims(
                core::slice::from_ref(&request.claim),
                request.signed_max_holders,
            )?;
            self.external_nics.insert(
                key.clone(),
                AuthorityEntry {
                    holders: vec![Holder {
                        claim: request.claim,
                        owner_proof: request.owner_proof.clone(),
                    }],
                    signed_max_holders: request.signed_max_holders,
                },
            );
        }
        Ok(ExternalNicLease {
            key,
            owner_proof: request.owner_proof,
        })
    }

    fn release(&mut self, lease: &ExternalNicLease) -> Result<(), AuthorityError> {
        let entry = self
            .external_nics
            .get_mut(&lease.key)
            .ok_or(AuthorityError::UnknownClaim)?;
        let holder = entry
            .holders
            .iter()
            .position(|holder| holder.owner_proof == lease.owner_proof)
            .ok_or(AuthorityError::OwnerProofMismatch)?;
        entry.holders.remove(holder);
        if entry.holders.is_empty() {
            self.external_nics.remove(&lease.key);
        }
        Ok(())
    }
}

impl core::fmt::Debug for HostGlobalAuthorityIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostGlobalAuthorityIndex")
            .field("authority_count", &self.external_nics.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn identity(value: &[u8]) -> ResolvedExternalNicIdentity {
        ResolvedExternalNicIdentity::from_trusted_inventory(value).unwrap()
    }

    fn proof(value: &str, generation: u64) -> ExternalNicOwnerProof {
        ExternalNicOwnerProof::new(uid(value), ResourceGeneration::new(generation).unwrap())
    }

    fn request(
        host: &ResourceUid,
        nic: &ResolvedExternalNicIdentity,
        zone: &ResourceUid,
        owner: ExternalNicOwnerProof,
        mode: MacvtapMode,
        policy: SharingPolicy,
        limit: usize,
    ) -> ExternalNicClaimRequest {
        ExternalNicClaimRequest::new(
            host.clone(),
            nic.clone(),
            ExternalNicClaim::new(zone.clone(), mode, policy),
            owner,
            limit,
        )
        .unwrap()
    }

    #[test]
    fn two_selectors_resolving_to_one_nic_share_one_host_global_key() {
        let mut inventory = TrustedExternalNicInventory::default();
        let resolved = identity(b"stable-inventory-identity");
        inventory
            .insert(IfName::parse("eno1").unwrap(), resolved.clone())
            .unwrap();
        inventory
            .insert(IfName::parse("uplink0").unwrap(), resolved.clone())
            .unwrap();
        let first = inventory.resolve(&IfName::parse("eno1").unwrap()).unwrap();
        let second = inventory
            .resolve(&IfName::parse("uplink0").unwrap())
            .unwrap();
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        assert_eq!(
            ExternalNicAuthorityKey::derive(host.clone(), &first),
            ExternalNicAuthorityKey::derive(host, &second)
        );
    }

    #[test]
    fn cross_zone_bridge_rejection_is_distinct_and_runs_no_effect() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let work = uid("223e4567-e89b-42d3-a456-426614174001");
        let personal = uid("323e4567-e89b-42d3-a456-426614174002");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        let first = request(
            &host,
            &nic,
            &work,
            proof("423e4567-e89b-42d3-a456-426614174003", 1),
            MacvtapMode::Bridge,
            SharingPolicy::Multiplexed,
            8,
        );
        index
            .admit_before_effect(first, |_| ExternalNicEffectOutcome::Confirmed)
            .unwrap();

        let mut effects = 0;
        let second = request(
            &host,
            &nic,
            &personal,
            proof("523e4567-e89b-42d3-a456-426614174004", 1),
            MacvtapMode::Bridge,
            SharingPolicy::Exclusive,
            1,
        );
        let error = index
            .admit_before_effect(second, |_| {
                effects += 1;
                ExternalNicEffectOutcome::Confirmed
            })
            .unwrap_err();
        assert_eq!(
            error,
            AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2)
        );
        assert_eq!(error.code(), "external-physical-nic-cross-zone-l2");
        assert_eq!(effects, 0);
    }

    #[test]
    fn same_zone_compatible_bridge_multiplex_obeys_the_signed_limit() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        for owner in [
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
        ] {
            index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof(owner, 1),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed,
                        2,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap();
        }
        let status = index.external_nic_status(host, &nic).unwrap();
        assert_eq!(status.holder_count(), 2);
        assert_eq!(status.arbitration(), SharingPolicy::Multiplexed);
        assert!(!status.available());
    }

    #[test]
    fn exclusive_mixed_and_non_bridge_claims_report_the_general_conflict() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        for (first_mode, first_policy, next_mode, next_policy) in [
            (
                MacvtapMode::Bridge,
                SharingPolicy::Exclusive,
                MacvtapMode::Bridge,
                SharingPolicy::Multiplexed,
            ),
            (
                MacvtapMode::Private,
                SharingPolicy::Exclusive,
                MacvtapMode::Private,
                SharingPolicy::Exclusive,
            ),
        ] {
            let nic = identity(b"one-physical-nic");
            let mut index = HostGlobalAuthorityIndex::default();
            index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof("323e4567-e89b-42d3-a456-426614174002", 1),
                        first_mode,
                        first_policy,
                        8,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap();
            let error = index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof("423e4567-e89b-42d3-a456-426614174003", 1),
                        next_mode,
                        next_policy,
                        8,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap_err();
            assert_eq!(
                error,
                AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
            );
        }

        let nic = identity(b"cross-zone-exclusive-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    proof("323e4567-e89b-42d3-a456-426614174002", 1),
                    MacvtapMode::Passthru,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        let mut effects = 0;
        let error = index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &uid("523e4567-e89b-42d3-a456-426614174004"),
                    proof("423e4567-e89b-42d3-a456-426614174003", 1),
                    MacvtapMode::Passthru,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| {
                    effects += 1;
                    ExternalNicEffectOutcome::Confirmed
                },
            )
            .unwrap_err();
        assert_eq!(
            error,
            AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        );
        assert_eq!(effects, 0);
    }

    #[test]
    fn restart_adopts_one_exact_owner_and_quarantines_ambiguity() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let owner = proof("323e4567-e89b-42d3-a456-426614174002", 4);
        let mut index = HostGlobalAuthorityIndex::default();
        index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    owner.clone(),
                    MacvtapMode::Bridge,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        assert!(matches!(
            index.adopt(host.clone(), &nic, &owner, core::slice::from_ref(&owner)),
            ExternalNicAdoption::Adopted(_)
        ));
        assert!(matches!(
            index.adopt(host, &nic, &owner, &[owner.clone(), owner.clone()]),
            ExternalNicAdoption::QuarantinedAmbiguous
        ));
    }

    #[test]
    fn update_and_delete_release_only_after_attachment_close() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        let gate = index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    proof("323e4567-e89b-42d3-a456-426614174002", 1),
                    MacvtapMode::Bridge,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        let lease = gate.into_lease();
        assert_eq!(
            index.close_then_release(&lease, || ExternalNicCloseOutcome::RetryableFailure),
            Err(AuthorityError::AttachmentCloseUnconfirmed)
        );
        assert!(index.external_nic_status(host.clone(), &nic).is_some());

        let adopted = match index.adopt(
            host.clone(),
            &nic,
            &proof("323e4567-e89b-42d3-a456-426614174002", 1),
            &[proof("323e4567-e89b-42d3-a456-426614174002", 1)],
        ) {
            ExternalNicAdoption::Adopted(lease) => lease,
            other => panic!("expected adoption, got {other:?}"),
        };
        let mut closed = false;
        let replacement = request(
            &host,
            &nic,
            &zone,
            proof("423e4567-e89b-42d3-a456-426614174003", 2),
            MacvtapMode::Bridge,
            SharingPolicy::Exclusive,
            1,
        );
        let replacement_lease = index
            .replace_after_close(&adopted, replacement, || {
                closed = true;
                ExternalNicCloseOutcome::Confirmed
            })
            .unwrap();
        assert!(closed);
        index
            .close_then_release(&replacement_lease, || ExternalNicCloseOutcome::Confirmed)
            .unwrap();
        assert!(index.external_nic_status(host, &nic).is_none());
    }

    #[test]
    fn diagnostics_never_expose_identity_digest_host_or_owner_values() {
        let identity_canary = b"private-hardware-identity";
        let host_canary = "123e4567-e89b-42d3-a456-426614174000";
        let owner_canary = "223e4567-e89b-42d3-a456-426614174001";
        let nic = identity(identity_canary);
        let owner = proof(owner_canary, 1);
        let key = ExternalNicAuthorityKey::derive(uid(host_canary), &nic);
        let rendered = format!("{nic:?} {owner:?} {key:?}");
        for canary in [
            String::from_utf8(identity_canary.to_vec()).unwrap(),
            host_canary.to_owned(),
            owner_canary.to_owned(),
            key.opaque_digest.clone(),
        ] {
            assert!(!rendered.contains(&canary));
        }
    }
}
