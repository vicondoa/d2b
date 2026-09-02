use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::{
    ResourceBundleGenerationId, ResourceUid, ZoneId, storage::ZoneStoreStorageRow,
};
use d2b_contracts_zone_session::v3::resource_bundle::{
    RESOURCE_BUNDLE_SCHEMA_VERSION, RESOURCE_BUNDLE_VERSION, ResourceBundle,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core_controller::coordinator::{CoordinatorError, ZoneCoordinator};
use sha2::{Digest, Sha256};

/// Prefix reserved for the durable all-Zone publication operation.
pub const ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX: &str = "zone-generation-publication:";

/// Closed failure while binding a Zone bundle to its immutable store identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneAuthorityError {
    /// The candidate bundle does not use the accepted v3 contract versions.
    ContractVersionMismatch,
    /// The bundle names a different Zone than the authority entry.
    ZoneMismatch,
    /// The bundle omitted its immutable Zone UID.
    ZoneUidMissing,
    /// The bundle and storage row disagree about the Zone UID.
    ZoneUidMismatch,
    /// The storage row does not use the canonical per-Zone store id.
    StoreIdMismatch,
    /// The bundle content hash is not a valid generation identity.
    BundleGenerationInvalid,
    /// The local Zone set is empty.
    EmptyZoneSet,
    /// The candidate generation does not cover exactly the local Zone set.
    IncompleteGeneration,
}

impl ZoneAuthorityError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ContractVersionMismatch => "zone-authority-contract-version-mismatch",
            Self::ZoneMismatch => "zone-authority-zone-mismatch",
            Self::ZoneUidMissing => "zone-authority-zone-uid-missing",
            Self::ZoneUidMismatch => "zone-authority-zone-uid-mismatch",
            Self::StoreIdMismatch => "zone-authority-store-id-mismatch",
            Self::BundleGenerationInvalid => "zone-authority-bundle-generation-invalid",
            Self::EmptyZoneSet => "zone-authority-zone-set-empty",
            Self::IncompleteGeneration => "zone-authority-generation-incomplete",
        }
    }
}

impl core::fmt::Display for ZoneAuthorityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for ZoneAuthorityError {}

/// Immutable identity used by one Zone runtime and its private effects.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneAuthorityIdentity {
    zone_uid: ResourceUid,
    store_uid: ResourceUid,
    store_epoch: u64,
    bundle_generation: ResourceBundleGenerationId,
}

impl core::fmt::Debug for ZoneAuthorityIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneAuthorityIdentity")
            .field("zone_uid", &self.zone_uid)
            .field("store_uid", &self.store_uid)
            .field("store_epoch", &self.store_epoch)
            .field("bundle_generation", &self.bundle_generation)
            .finish()
    }
}

impl ZoneAuthorityIdentity {
    /// Bind one verified bundle to one verified storage row.
    pub fn from_bundle_and_storage(
        zone: &ZoneId,
        bundle: &ResourceBundle,
        storage: &ZoneStoreStorageRow,
    ) -> Result<Self, ZoneAuthorityError> {
        validate_exact_contract_versions(bundle)?;
        if &bundle.zone != zone {
            return Err(ZoneAuthorityError::ZoneMismatch);
        }
        let zone_uid = bundle
            .zone_uid()
            .ok_or(ZoneAuthorityError::ZoneUidMissing)?;
        if zone_uid != storage.identity.zone_uid() {
            return Err(ZoneAuthorityError::ZoneUidMismatch);
        }
        if storage.zone_store_id.as_str() != format!("zone-store-{}", zone.as_str()) {
            return Err(ZoneAuthorityError::StoreIdMismatch);
        }
        let bundle_generation =
            ResourceBundleGenerationId::parse(bundle.integrity().content_hash.clone())
                .map_err(|_| ZoneAuthorityError::BundleGenerationInvalid)?;
        Ok(Self {
            zone_uid: zone_uid.clone(),
            store_uid: storage.identity.store_uid().clone(),
            store_epoch: storage.identity.store_epoch(),
            bundle_generation,
        })
    }

    /// Borrow the immutable Zone UID.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the immutable store UID.
    pub const fn store_uid(&self) -> &ResourceUid {
        &self.store_uid
    }

    /// Return the store identity epoch.
    pub const fn store_epoch(&self) -> u64 {
        self.store_epoch
    }

    /// Borrow the content-addressed bundle generation.
    pub const fn bundle_generation(&self) -> &ResourceBundleGenerationId {
        &self.bundle_generation
    }
}

/// Verify the exact contract tuple accepted before any store or effect path.
pub fn validate_exact_contract_versions(bundle: &ResourceBundle) -> Result<(), ZoneAuthorityError> {
    if bundle.schema_version != RESOURCE_BUNDLE_SCHEMA_VERSION
        || bundle.bundle_version != RESOURCE_BUNDLE_VERSION
    {
        return Err(ZoneAuthorityError::ContractVersionMismatch);
    }
    bundle
        .verify()
        .map_err(|_| ZoneAuthorityError::ContractVersionMismatch)
}

/// Derive one durable identity for the complete local Zone generation set.
///
/// The set digest is used as the claim identity of the store-backed
/// publication operation.  It is deliberately derived from the sorted Zone
/// map rather than held in a process-local coordinator, so a restart can
/// resume the same prepared set from the store operation ledger.
pub fn complete_generation_set_digest(
    zones: &BTreeSet<ZoneId>,
    generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
) -> Result<ResourceBundleGenerationId, ZoneAuthorityError> {
    if zones.is_empty() {
        return Err(ZoneAuthorityError::EmptyZoneSet);
    }
    if generations.keys().cloned().collect::<BTreeSet<_>>() != *zones {
        return Err(ZoneAuthorityError::IncompleteGeneration);
    }

    let mut digest = Sha256::new();
    digest.update(b"d2b:v3:zone-generation-set\0");
    for (zone, generation) in generations {
        digest.update(zone.as_str().as_bytes());
        digest.update([0]);
        digest.update(generation.as_str().as_bytes());
        digest.update([0]);
    }
    ResourceBundleGenerationId::parse(format!("sha256:{:x}", digest.finalize()))
        .map_err(|_| ZoneAuthorityError::BundleGenerationInvalid)
}

pub fn new_coordinator() -> Arc<Mutex<ZoneCoordinator>> {
    Arc::new(Mutex::new(ZoneCoordinator::new()))
}

pub fn authoritative_zone_ids(resolver: &BundleResolver) -> Result<BTreeSet<ZoneId>, &'static str> {
    let zones = resolver.zone_resource_bundle_zones()?;
    if zones.is_empty() {
        return Err("bundle Zone resource bundle index empty");
    }
    Ok(zones)
}

pub fn register_authoritative_zones(
    coordinator: &Arc<Mutex<ZoneCoordinator>>,
    resolver: &BundleResolver,
) -> Result<(), &'static str> {
    #[allow(unused_mut)]
    let mut coordinator = coordinator
        .lock()
        .map_err(|_| "zone coordinator lock unavailable")?;
    for zone in authoritative_zone_ids(resolver)? {
        let _ = coordinator.register_zone(zone);
    }
    for runtime in &resolver.host.vm_runtimes {
        let Some(environment) = runtime.env.as_deref() else {
            continue;
        };
        let zone = ZoneId::parse(environment).map_err(|_| "VM Zone identity invalid")?;
        coordinator
            .bind_vm(runtime.vm.clone(), &zone)
            .map_err(|_| "VM Zone binding unavailable")?;
    }
    for (vm, runtime) in &resolver.manifest.vms {
        let Some(environment) = runtime.env.as_deref() else {
            continue;
        };
        let zone = ZoneId::parse(environment).map_err(|_| "manifest Zone identity invalid")?;
        coordinator
            .bind_vm(vm.clone(), &zone)
            .map_err(|_| "manifest VM Zone binding unavailable")?;
    }
    Ok(())
}

pub fn authoritative_zone_for_vm(
    coordinator: &Arc<Mutex<ZoneCoordinator>>,
    vm: &str,
) -> Result<ZoneId, CoordinatorError> {
    #[allow(unused_mut)]
    let mut coordinator = coordinator
        .lock()
        .map_err(|_| CoordinatorError::ZoneNotRegistered)?;
    if let Ok(zone) = coordinator.zone_for_vm(vm) {
        return Ok(zone.clone());
    }

    #[cfg(any(test, feature = "test-support"))]
    {
        let zone = ZoneId::parse(vm).map_err(|_| CoordinatorError::VmNotRegistered)?;
        coordinator.register_zone(zone.clone());
        coordinator.bind_vm(vm.to_owned(), &zone)?;
        Ok(zone)
    }

    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = vm;
        Err(CoordinatorError::VmNotRegistered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::storage::ZoneStoreIdentity;
    use std::collections::BTreeMap;

    fn digest(byte: char) -> ResourceBundleGenerationId {
        ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64)))
            .expect("generation")
    }

    fn storage_identity(suffix: char) -> ZoneStoreIdentity {
        ZoneStoreIdentity::new(
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-42661417400{suffix}"))
                .expect("zone uid"),
            ResourceUid::parse(format!("223e4567-e89b-42d3-a456-42661417400{suffix}"))
                .expect("store uid"),
            1,
        )
        .expect("valid identity")
    }

    fn storage_row(zone: &str, identity: &ZoneStoreIdentity) -> ZoneStoreStorageRow {
        serde_json::from_value(serde_json::json!({
            "identity": identity,
            "zoneStoreId": format!("zone-store-{zone}"),
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": format!("zone-store-parent-{zone}"),
            "ownership": {
                "owner": "d2b-zonert", "group": "d2b-zonert",
                "mode": "0640", "linkCount": 1
            },
            "auxiliaryDirectories": {
                "audit": {
                    "directoryId": format!("zone-store-audit-{zone}"),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                },
                "telemetry": {
                    "directoryId": format!("zone-store-telemetry-{zone}"),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                }
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": format!("zone-store-marker-{zone}")
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        }))
        .expect("valid storage row")
    }

    fn bundle(zone: &str, uid: &ResourceUid) -> ResourceBundle {
        ResourceBundle::new(
            ZoneId::parse(zone).unwrap(),
            Vec::new(),
            "sha256:".to_owned() + &"a".repeat(64),
            BTreeMap::new(),
            BTreeMap::new(),
            d2b_contracts_resource::v3::Timestamp::parse("2026-08-26T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(uid.clone())
    }

    #[test]
    fn identity_binding_keeps_two_level_zones_distinct() {
        let work = storage_identity('2');
        let personal = storage_identity('3');
        let work_bundle = bundle("work", work.zone_uid());
        let work_tuple = ZoneAuthorityIdentity::from_bundle_and_storage(
            &ZoneId::parse("work").unwrap(),
            &work_bundle,
            &storage_row("work", &work),
        )
        .expect("work identity");
        assert_eq!(work_tuple.zone_uid(), work.zone_uid());
        assert_ne!(work_tuple.zone_uid(), personal.zone_uid());
        assert_eq!(
            work_tuple.bundle_generation().as_str(),
            work_bundle.integrity().content_hash
        );
    }

    #[test]
    fn identity_tuple_mismatch_is_refused_before_store_access() {
        let work = storage_identity('2');
        let other = storage_identity('3');
        let bundle = bundle("work", work.zone_uid());
        let error = ZoneAuthorityIdentity::from_bundle_and_storage(
            &ZoneId::parse("work").unwrap(),
            &bundle,
            &storage_row("work", &other),
        )
        .expect_err("mismatched Zone identity must fail closed");
        assert_eq!(error, ZoneAuthorityError::ZoneUidMismatch);
    }

    #[test]
    fn old_bundle_contract_is_rejected_before_store_access() {
        let identity = storage_identity('2');
        let mut bundle = bundle("work", identity.zone_uid());
        bundle.schema_version = 2;
        assert_eq!(
            validate_exact_contract_versions(&bundle),
            Err(ZoneAuthorityError::ContractVersionMismatch)
        );
    }

    #[test]
    fn incomplete_generation_set_is_refused_before_digest_creation() {
        let zones: BTreeSet<_> = [
            ZoneId::parse("work").unwrap(),
            ZoneId::parse("personal").unwrap(),
        ]
        .into_iter()
        .collect();
        let error = complete_generation_set_digest(
            &zones,
            &BTreeMap::from([(ZoneId::parse("work").unwrap(), digest('a'))]),
        )
        .expect_err("missing local Zone must fail closed");
        assert_eq!(error, ZoneAuthorityError::IncompleteGeneration);
    }

    #[test]
    fn generation_set_digest_is_order_independent_and_changes_on_any_zone() {
        let zones: BTreeSet<_> = [
            ZoneId::parse("work").unwrap(),
            ZoneId::parse("personal").unwrap(),
        ]
        .into_iter()
        .collect();
        let first = BTreeMap::from([
            (ZoneId::parse("work").unwrap(), digest('a')),
            (ZoneId::parse("personal").unwrap(), digest('a')),
        ]);
        let reordered = BTreeMap::from([
            (ZoneId::parse("personal").unwrap(), digest('a')),
            (ZoneId::parse("work").unwrap(), digest('a')),
        ]);
        let changed = BTreeMap::from([
            (ZoneId::parse("work").unwrap(), digest('b')),
            (ZoneId::parse("personal").unwrap(), digest('b')),
        ]);
        assert_eq!(
            complete_generation_set_digest(&zones, &first),
            complete_generation_set_digest(&zones, &reordered)
        );
        assert_ne!(
            complete_generation_set_digest(&zones, &first),
            complete_generation_set_digest(&zones, &changed)
        );
    }
}
