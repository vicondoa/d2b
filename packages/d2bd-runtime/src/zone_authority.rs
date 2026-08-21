use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::ZoneId;
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core_controller::coordinator::{CoordinatorError, ZoneCoordinator};

pub fn new_coordinator() -> Arc<Mutex<ZoneCoordinator>> {
    Arc::new(Mutex::new(ZoneCoordinator::new()))
}

pub fn authoritative_zone_ids(resolver: &BundleResolver) -> Result<BTreeSet<ZoneId>, &'static str> {
    let Some(artifact_hashes) = resolver.bundle.artifact_hashes.as_ref() else {
        return Err("bundle Zone storage artifact index unavailable");
    };
    let mut zones = BTreeSet::new();
    for key in artifact_hashes.keys() {
        let Some(storage_path) = key.strip_prefix("zones/") else {
            continue;
        };
        let Some(zone_name) = storage_path.strip_suffix("/storage.json") else {
            continue;
        };
        if zone_name.is_empty() || zone_name.contains('/') || zone_name.contains('\\') {
            return Err("bundle Zone storage artifact index invalid");
        }
        let zone =
            ZoneId::parse(zone_name.to_owned()).map_err(|_| "bundle Zone identity invalid")?;
        zones.insert(zone);
    }
    if zones.is_empty() {
        return Err("bundle Zone storage artifact index empty");
    }
    if !zones.iter().any(|zone| zone.as_str() == "local-root") {
        return Err("bundle Zone storage artifact index missing local root");
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
