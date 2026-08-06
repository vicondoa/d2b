//! Zone-wide quota admission and drain authority.
//!
//! The contract crate owns the wire shape.  This module owns the in-memory
//! admission index: it reserves the one Quota authority for a Zone, checks
//! resource usage before a create effect, and keeps a dependent resource from
//! being silently reassigned during deletion.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    QuotaEnforcementPolicy, QuotaSpec, QuotaStatusResource, ResourceRef, ResourceTypeName,
    ResourceUid, Timestamp,
};

use super::{
    AuthorityError, AuthorityLease, AuthorityOwnerProof, AuthorityRequest, HostGlobalAuthorityIndex,
};

/// Closed quota metric label keys.
pub const QUOTA_METRIC_LABEL_KEYS: &[&str] = &["outcome", "dimension"];

/// Resource quantities checked by a Quota admission.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaResourceRequest {
    cpu: u64,
    memory_mib: u64,
    storage_gib: u64,
}

impl QuotaResourceRequest {
    /// Construct a nonnegative resource request.
    pub const fn new(cpu: u64, memory_mib: u64, storage_gib: u64) -> Self {
        Self {
            cpu,
            memory_mib,
            storage_gib,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotaDecisionKind {
    Admitted,
    AdmittedSoftOverage,
}

/// Result of one quota admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaAdmission {
    kind: QuotaDecisionKind,
}

impl QuotaAdmission {
    /// Whether the resource was admitted under a soft overage.
    pub const fn is_soft_overage(self) -> bool {
        matches!(self.kind, QuotaDecisionKind::AdmittedSoftOverage)
    }
}

/// Closed quota-controller refusal reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaError {
    /// The scope authority or generic authority request failed.
    Authority(AuthorityError),
    /// The resource reference is not a valid quota subject.
    InvalidResourceReference,
    /// The ResourceType entry is not valid for the current admission.
    InvalidResourceType,
    /// The exact resource already has a reservation.
    DuplicateReservation,
    /// A hard ceiling would be exceeded.
    QuotaExceeded,
    /// A checked aggregate would overflow its bounded counter.
    ArithmeticOverflow,
    /// The exact reservation does not exist.
    UnknownReservation,
    /// The Quota is draining and cannot admit new dependents.
    DrainPending,
    /// Deletion cannot complete while quotaRef dependents remain.
    DependentsRemain,
    /// The Quota status projection cannot fit its bounded contract.
    StatusInvalid,
}

impl QuotaError {
    /// Return the stable identity-free failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Authority(error) => error.code(),
            Self::InvalidResourceReference => "quota-resource-reference-invalid",
            Self::InvalidResourceType => "quota-resource-type-invalid",
            Self::DuplicateReservation => "quota-reservation-duplicate",
            Self::QuotaExceeded => "quota-exceeded",
            Self::ArithmeticOverflow => "quota-arithmetic-overflow",
            Self::UnknownReservation => "quota-reservation-unknown",
            Self::DrainPending => "quota-drain-pending",
            Self::DependentsRemain => "quota-dependents-remain",
            Self::StatusInvalid => "quota-status-invalid",
        }
    }
}

impl core::fmt::Display for QuotaError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for QuotaError {}

impl From<AuthorityError> for QuotaError {
    fn from(value: AuthorityError) -> Self {
        Self::Authority(value)
    }
}

struct Reservation {
    resource_type: ResourceTypeName,
    request: QuotaResourceRequest,
    has_quota_ref: bool,
}

#[derive(Default)]
struct Usage {
    resources: u64,
    cpu: u64,
    memory_mib: u64,
    storage_gib: u64,
    by_type: BTreeMap<ResourceTypeName, u64>,
    reservations: BTreeMap<ResourceRef, Reservation>,
}

/// Core-owned Quota handler for one Zone scope.
pub struct QuotaAuthority {
    zone_uid: ResourceUid,
    spec: QuotaSpec,
    lease: AuthorityLease,
    usage: Usage,
    deletion_requested: bool,
    deleted: bool,
    last_checked_at: Option<Timestamp>,
}

impl core::fmt::Debug for QuotaAuthority {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("QuotaAuthority")
            .field("has_zone", &true)
            .field("reservation_count", &self.usage.reservations.len())
            .field("deletion_requested", &self.deletion_requested)
            .field("deleted", &self.deleted)
            .finish()
    }
}

impl QuotaAuthority {
    /// Admit the exactly-one Quota scope before any resource effect.
    pub fn admit(
        index: &mut HostGlobalAuthorityIndex,
        zone_uid: ResourceUid,
        spec: QuotaSpec,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, QuotaError> {
        if spec.scope() != d2b_contracts::v3::QuotaScope::Zone {
            return Err(QuotaError::InvalidResourceType);
        }
        let lease =
            index.admit_authority(AuthorityRequest::quota(zone_uid.clone(), owner_proof)?)?;
        Ok(Self {
            zone_uid,
            spec,
            lease,
            usage: Usage::default(),
            deletion_requested: false,
            deleted: false,
            last_checked_at: None,
        })
    }

    /// Borrow the Zone identity used only by the trusted controller.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the admitted Quota specification.
    pub const fn spec(&self) -> &QuotaSpec {
        &self.spec
    }

    /// Admit a resource before its create effect.
    ///
    /// `has_quota_ref` records whether CPU, memory, and storage are governed
    /// by this Quota.  Every resource still counts toward the aggregate and
    /// per-type resource-count ceilings.
    pub fn admit_resource(
        &mut self,
        resource_ref: ResourceRef,
        resource_type: ResourceTypeName,
        request: QuotaResourceRequest,
        has_quota_ref: bool,
    ) -> Result<QuotaAdmission, QuotaError> {
        if self.deleted || self.deletion_requested {
            return Err(QuotaError::DrainPending);
        }
        if resource_ref.resource_type().as_str() == "Quota"
            || resource_ref.resource_type().as_str() == "EmergencyPolicy"
        {
            return Err(QuotaError::InvalidResourceReference);
        }
        if resource_ref.resource_type() != &resource_type {
            return Err(QuotaError::InvalidResourceType);
        }
        if self.usage.reservations.contains_key(&resource_ref) {
            return Err(QuotaError::DuplicateReservation);
        }

        let next_resources = self
            .usage
            .resources
            .checked_add(1)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        let current_type = self
            .usage
            .by_type
            .get(&resource_type)
            .copied()
            .unwrap_or_default();
        let next_type = current_type
            .checked_add(1)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        let ceilings = self.spec.ceilings();
        let type_ceiling = self
            .spec
            .per_type_ceilings()
            .get(&resource_type)
            .and_then(|ceiling| ceiling.max_resources())
            .unwrap_or(ceilings.max_resources_per_type());
        let mut over_quota = next_resources > ceilings.max_resources() || next_type > type_ceiling;

        let (next_cpu, next_memory, next_storage) = if has_quota_ref {
            let next_cpu = self
                .usage
                .cpu
                .checked_add(request.cpu)
                .ok_or(QuotaError::ArithmeticOverflow)?;
            let next_memory = self
                .usage
                .memory_mib
                .checked_add(request.memory_mib)
                .ok_or(QuotaError::ArithmeticOverflow)?;
            let next_storage = self
                .usage
                .storage_gib
                .checked_add(request.storage_gib)
                .ok_or(QuotaError::ArithmeticOverflow)?;
            if let Some(limit) = ceilings.max_cpu() {
                over_quota |= next_cpu > limit;
            }
            if let Some(limit) = ceilings.max_memory_mib() {
                over_quota |= next_memory > limit;
            }
            if let Some(limit) = ceilings.max_storage_gib() {
                over_quota |= next_storage > limit;
            }
            if let Some(type_ceiling) = self.spec.per_type_ceilings().get(&resource_type) {
                if let Some(limit) = type_ceiling.max_cpu() {
                    over_quota |= next_cpu > limit;
                }
                if let Some(limit) = type_ceiling.max_memory_mib() {
                    over_quota |= next_memory > limit;
                }
                if let Some(limit) = type_ceiling.max_storage_gib() {
                    over_quota |= next_storage > limit;
                }
            }
            (next_cpu, next_memory, next_storage)
        } else {
            (
                self.usage.cpu,
                self.usage.memory_mib,
                self.usage.storage_gib,
            )
        };

        if over_quota && self.spec.enforcement_policy() == QuotaEnforcementPolicy::Hard {
            return Err(QuotaError::QuotaExceeded);
        }

        self.usage.resources = next_resources;
        self.usage.cpu = next_cpu;
        self.usage.memory_mib = next_memory;
        self.usage.storage_gib = next_storage;
        self.usage.by_type.insert(resource_type.clone(), next_type);
        self.usage.reservations.insert(
            resource_ref,
            Reservation {
                resource_type,
                request,
                has_quota_ref,
            },
        );
        Ok(QuotaAdmission {
            kind: if over_quota {
                QuotaDecisionKind::AdmittedSoftOverage
            } else {
                QuotaDecisionKind::Admitted
            },
        })
    }

    /// Release an exact resource reservation after its delete commit.
    pub fn release_resource(&mut self, resource_ref: &ResourceRef) -> Result<(), QuotaError> {
        let reservation = self
            .usage
            .reservations
            .remove(resource_ref)
            .ok_or(QuotaError::UnknownReservation)?;
        self.usage.resources = self
            .usage
            .resources
            .checked_sub(1)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        if reservation.has_quota_ref {
            self.usage.cpu = self
                .usage
                .cpu
                .checked_sub(reservation.request.cpu)
                .ok_or(QuotaError::ArithmeticOverflow)?;
            self.usage.memory_mib = self
                .usage
                .memory_mib
                .checked_sub(reservation.request.memory_mib)
                .ok_or(QuotaError::ArithmeticOverflow)?;
            self.usage.storage_gib = self
                .usage
                .storage_gib
                .checked_sub(reservation.request.storage_gib)
                .ok_or(QuotaError::ArithmeticOverflow)?;
        }
        let remove_type =
            if let Some(count) = self.usage.by_type.get_mut(&reservation.resource_type) {
                *count = count.checked_sub(1).ok_or(QuotaError::ArithmeticOverflow)?;
                *count == 0
            } else {
                return Err(QuotaError::ArithmeticOverflow);
            };
        if remove_type {
            self.usage.by_type.remove(&reservation.resource_type);
        }
        Ok(())
    }

    /// Mark the Quota for deletion and retain its drain finalizer if needed.
    pub fn request_delete(&mut self) {
        self.deletion_requested = true;
    }

    /// Whether the core quota-drain finalizer is still required.
    pub fn drain_pending(&self) -> bool {
        self.deletion_requested && !self.usage.reservations.is_empty()
    }

    /// Return the number of active quotaRef dependents.
    pub fn dependent_count(&self) -> usize {
        self.usage
            .reservations
            .values()
            .filter(|reservation| reservation.has_quota_ref)
            .count()
    }

    /// Complete deletion only after every quotaRef dependent is gone.
    pub fn complete_delete(
        &mut self,
        index: &mut HostGlobalAuthorityIndex,
    ) -> Result<(), QuotaError> {
        if !self.deletion_requested {
            return Err(QuotaError::DrainPending);
        }
        if self.dependent_count() != 0 {
            return Err(QuotaError::DependentsRemain);
        }
        index.release_authority(&self.lease)?;
        self.deleted = true;
        Ok(())
    }

    /// Record the latest bounded observation time.
    pub fn mark_checked_at(&mut self, timestamp: Timestamp) {
        self.last_checked_at = Some(timestamp);
    }

    /// Build the ResourceType-common status projection.
    pub fn status(&self) -> Result<QuotaStatusResource, QuotaError> {
        let used_resources =
            u32::try_from(self.usage.resources).map_err(|_| QuotaError::StatusInvalid)?;
        let used_cpu = self.optional_u32(self.usage.cpu, self.spec.ceilings().max_cpu())?;
        let used_memory =
            self.optional_u32(self.usage.memory_mib, self.spec.ceilings().max_memory_mib())?;
        let used_storage = self.optional_u32(
            self.usage.storage_gib,
            self.spec.ceilings().max_storage_gib(),
        )?;
        let mut over_quota_types = self
            .usage
            .by_type
            .iter()
            .filter(|(resource_type, count)| self.type_over_quota(resource_type, **count))
            .map(|(resource_type, _)| resource_type.clone())
            .collect::<Vec<_>>();
        let mut over_quota = self.usage.resources > self.spec.ceilings().max_resources();
        over_quota |= over_quota_types.iter().any(|_| true);
        if let Some(limit) = self.spec.ceilings().max_cpu() {
            over_quota |= self.usage.cpu > limit;
        }
        if let Some(limit) = self.spec.ceilings().max_memory_mib() {
            over_quota |= self.usage.memory_mib > limit;
        }
        if let Some(limit) = self.spec.ceilings().max_storage_gib() {
            over_quota |= self.usage.storage_gib > limit;
        }
        over_quota_types.truncate(16);
        let dependent_count =
            u32::try_from(self.dependent_count()).map_err(|_| QuotaError::StatusInvalid)?;
        QuotaStatusResource::new(
            used_resources,
            used_cpu,
            used_memory,
            used_storage,
            over_quota,
            over_quota_types,
            self.last_checked_at.clone(),
            dependent_count,
        )
        .map_err(|_| QuotaError::StatusInvalid)
    }

    fn type_over_quota(&self, resource_type: &ResourceTypeName, count: u64) -> bool {
        let ceilings = self.spec.ceilings();
        let type_ceiling = self.spec.per_type_ceilings().get(resource_type);
        if count
            > type_ceiling
                .and_then(|ceiling| ceiling.max_resources())
                .unwrap_or(ceilings.max_resources_per_type())
        {
            return true;
        }
        let mut cpu: u64 = 0;
        let mut memory: u64 = 0;
        let mut storage: u64 = 0;
        for reservation in self.usage.reservations.values().filter(|reservation| {
            reservation.has_quota_ref && &reservation.resource_type == resource_type
        }) {
            cpu = match cpu.checked_add(reservation.request.cpu) {
                Some(value) => value,
                None => return true,
            };
            memory = match memory.checked_add(reservation.request.memory_mib) {
                Some(value) => value,
                None => return true,
            };
            storage = match storage.checked_add(reservation.request.storage_gib) {
                Some(value) => value,
                None => return true,
            };
        }
        type_ceiling.is_some_and(|ceiling| {
            ceiling.max_cpu().is_some_and(|limit| cpu > limit)
                || ceiling.max_memory_mib().is_some_and(|limit| memory > limit)
                || ceiling
                    .max_storage_gib()
                    .is_some_and(|limit| storage > limit)
        })
    }

    fn optional_u32(&self, value: u64, limit: Option<u64>) -> Result<Option<u32>, QuotaError> {
        limit
            .map(|_| u32::try_from(value).map_err(|_| QuotaError::StatusInvalid))
            .transpose()
    }
}

/// Short name used by the fixed core handler catalog.
pub type QuotaHandler = QuotaAuthority;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use d2b_contracts::v3::{QuotaCeilings, QuotaScope, QuotaTypeCeiling, ResourceGeneration};

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn owner(value: &str) -> AuthorityOwnerProof {
        AuthorityOwnerProof::new(uid(value), ResourceGeneration::new(1).unwrap())
    }

    fn spec(max_resources: u64, policy: QuotaEnforcementPolicy) -> QuotaSpec {
        QuotaSpec::new(
            QuotaCeilings::new(
                max_resources,
                max_resources,
                8,
                Some(10),
                Some(10),
                Some(10),
            )
            .unwrap(),
            BTreeMap::new(),
            QuotaScope::Zone,
            policy,
        )
        .unwrap()
    }

    fn resource(name: &str) -> ResourceRef {
        let rendered = format!("Guest/{name}");
        ResourceRef::parse(&rendered).unwrap()
    }

    #[test]
    fn only_one_quota_authority_is_admitted_per_zone() {
        let mut index = HostGlobalAuthorityIndex::default();
        QuotaAuthority::admit(
            &mut index,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            spec(2, QuotaEnforcementPolicy::Hard),
            owner("223e4567-e89b-42d3-a456-426614174001"),
        )
        .unwrap();
        let error = QuotaAuthority::admit(
            &mut index,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            spec(2, QuotaEnforcementPolicy::Hard),
            owner("323e4567-e89b-42d3-a456-426614174002"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "duplicateConflict");
    }

    #[test]
    fn hard_quota_rejects_before_mutation_and_soft_quota_warns() {
        let mut hard_index = HostGlobalAuthorityIndex::default();
        let mut hard = QuotaAuthority::admit(
            &mut hard_index,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            spec(1, QuotaEnforcementPolicy::Hard),
            owner("223e4567-e89b-42d3-a456-426614174001"),
        )
        .unwrap();
        hard.admit_resource(
            resource("first"),
            ResourceTypeName::parse("Guest").unwrap(),
            QuotaResourceRequest::new(1, 1, 1),
            true,
        )
        .unwrap();
        assert_eq!(
            hard.admit_resource(
                resource("second"),
                ResourceTypeName::parse("Guest").unwrap(),
                QuotaResourceRequest::new(1, 1, 1),
                true,
            ),
            Err(QuotaError::QuotaExceeded)
        );
        assert_eq!(hard.status().unwrap().used_resources(), 1);

        let mut soft_index = HostGlobalAuthorityIndex::default();
        let mut soft = QuotaAuthority::admit(
            &mut soft_index,
            uid("323e4567-e89b-42d3-a456-426614174002"),
            spec(1, QuotaEnforcementPolicy::Soft),
            owner("423e4567-e89b-42d3-a456-426614174003"),
        )
        .unwrap();
        soft.admit_resource(
            resource("first"),
            ResourceTypeName::parse("Guest").unwrap(),
            QuotaResourceRequest::default(),
            false,
        )
        .unwrap();
        assert!(
            soft.admit_resource(
                resource("second"),
                ResourceTypeName::parse("Guest").unwrap(),
                QuotaResourceRequest::default(),
                false,
            )
            .unwrap()
            .is_soft_overage()
        );
        assert!(soft.status().unwrap().over_quota());
    }

    #[test]
    fn quota_drain_never_reassigns_dependents_or_clears_early() {
        let mut index = HostGlobalAuthorityIndex::default();
        let mut quota = QuotaAuthority::admit(
            &mut index,
            uid("523e4567-e89b-42d3-a456-426614174004"),
            spec(4, QuotaEnforcementPolicy::Hard),
            owner("623e4567-e89b-42d3-a456-426614174005"),
        )
        .unwrap();
        quota
            .admit_resource(
                resource("dependent"),
                ResourceTypeName::parse("Guest").unwrap(),
                QuotaResourceRequest::default(),
                true,
            )
            .unwrap();
        quota.request_delete();
        assert!(quota.drain_pending());
        assert_eq!(
            quota.complete_delete(&mut index),
            Err(QuotaError::DependentsRemain)
        );
        quota.release_resource(&resource("dependent")).unwrap();
        quota.complete_delete(&mut index).unwrap();
        assert_eq!(quota.dependent_count(), 0);
    }

    #[test]
    fn per_type_ceiling_is_checked_independently() {
        let mut per_type = BTreeMap::new();
        per_type.insert(
            ResourceTypeName::parse("Guest").unwrap(),
            QuotaTypeCeiling::new(Some(1), None, None, None).unwrap(),
        );
        let spec = QuotaSpec::new(
            QuotaCeilings::default(),
            per_type,
            QuotaScope::Zone,
            QuotaEnforcementPolicy::Hard,
        )
        .unwrap();
        let mut index = HostGlobalAuthorityIndex::default();
        let mut quota = QuotaAuthority::admit(
            &mut index,
            uid("723e4567-e89b-42d3-a456-426614174006"),
            spec,
            owner("823e4567-e89b-42d3-a456-426614174007"),
        )
        .unwrap();
        quota
            .admit_resource(
                resource("first"),
                ResourceTypeName::parse("Guest").unwrap(),
                QuotaResourceRequest::default(),
                false,
            )
            .unwrap();
        assert_eq!(
            quota.admit_resource(
                resource("second"),
                ResourceTypeName::parse("Guest").unwrap(),
                QuotaResourceRequest::default(),
                false,
            ),
            Err(QuotaError::QuotaExceeded)
        );
    }
}
