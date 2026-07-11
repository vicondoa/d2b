use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

use d2b_contracts::public_wire::{
    GraphicalLaunchPosture, WorkloadAvailability, WorkloadPublicSummary,
};
use d2b_core::{
    bundle_resolver::BundleResolver,
    configured_argv::ConfiguredArgv,
    realm_controller_config::RealmControllerPlacement,
    realm_workloads_launcher::LauncherWorkloadSummary,
    unsafe_local_workloads::UnsafeLocalLauncherItem,
    workload_identity::{WorkloadIdentity, WorkloadTarget},
};
use d2b_realm_core::{LauncherItemKind, ProtocolToken, WorkloadProviderKind, WorkloadState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkloadRoute {
    LocalVm { vm: String },
    UnsafeLocal,
    CapabilityUnavailable { provider: WorkloadProviderKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CatalogError {
    ArtifactsUnavailable,
    TargetNotFound,
    TargetAmbiguous,
    RealmNotDirectLocal,
    LauncherDisabled,
    ItemNotFound,
    ConfiguredItemMissing,
    ConfiguredItemMismatch,
    OperationConflict,
    OperationInProgress,
}

#[derive(Debug, Clone)]
pub(crate) struct CatalogEntry {
    pub metadata: LauncherWorkloadSummary,
    pub route: WorkloadRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchLedgerBegin {
    New,
    AlreadyCommitted,
}

#[derive(Debug, Clone)]
struct LaunchLedgerEntry {
    fingerprint: String,
    committed: bool,
    sequence: u64,
}

fn launch_ledger() -> &'static Mutex<BTreeMap<(u32, String), LaunchLedgerEntry>> {
    static LEDGER: OnceLock<Mutex<BTreeMap<(u32, String), LaunchLedgerEntry>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn begin_launch(
    requester_uid: u32,
    operation_id: &str,
    target: &WorkloadTarget,
    item_id: &ProtocolToken,
) -> Result<LaunchLedgerBegin, CatalogError> {
    let key = (requester_uid, operation_id.to_owned());
    let fingerprint = format!("{}:{}", target.to_canonical(), item_id.as_str());
    let mut ledger = launch_ledger().lock().expect("workload launch ledger");
    if let Some(entry) = ledger.get(&key) {
        if entry.fingerprint != fingerprint {
            return Err(CatalogError::OperationConflict);
        }
        return if entry.committed {
            Ok(LaunchLedgerBegin::AlreadyCommitted)
        } else {
            Err(CatalogError::OperationInProgress)
        };
    }
    if ledger.len() >= 1024 {
        let oldest = ledger
            .iter()
            .filter(|(_, entry)| entry.committed)
            .min_by_key(|(_, entry)| entry.sequence)
            .map(|(key, _)| key.clone())
            .ok_or(CatalogError::OperationInProgress)?;
        ledger.remove(&oldest);
    }
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    ledger.insert(
        key,
        LaunchLedgerEntry {
            fingerprint,
            committed: false,
            sequence: SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        },
    );
    Ok(LaunchLedgerBegin::New)
}

pub(crate) fn complete_launch(requester_uid: u32, operation_id: &str) {
    if let Some(entry) = launch_ledger()
        .lock()
        .expect("workload launch ledger")
        .get_mut(&(requester_uid, operation_id.to_owned()))
    {
        entry.committed = true;
    }
}

pub(crate) fn abort_launch(requester_uid: u32, operation_id: &str) {
    launch_ledger()
        .lock()
        .expect("workload launch ledger")
        .remove(&(requester_uid, operation_id.to_owned()));
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedExec {
    pub identity: WorkloadIdentity,
    pub route: WorkloadRoute,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkloadCatalog {
    entries: BTreeMap<String, CatalogEntry>,
    aliases: BTreeMap<String, Vec<String>>,
}

impl WorkloadCatalog {
    pub(crate) fn from_resolver(resolver: &BundleResolver) -> Result<Self, CatalogError> {
        let public = resolver
            .realm_workloads_launcher_v2
            .as_ref()
            .ok_or(CatalogError::ArtifactsUnavailable)?;
        let mut entries = BTreeMap::new();
        let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for metadata in public
            .workloads
            .iter()
            .filter(|workload| workload.launcher_enabled)
        {
            if !realm_is_direct_local(resolver, &metadata.identity) {
                continue;
            }
            let canonical = metadata.identity.canonical_target.to_canonical();
            aliases
                .entry(metadata.identity.workload_id.as_str().to_owned())
                .or_default()
                .push(canonical.clone());
            let route = match metadata.provider_kind {
                WorkloadProviderKind::LocalVm => metadata
                    .identity
                    .legacy_vm_name
                    .as_ref()
                    .map(|vm| WorkloadRoute::LocalVm {
                        vm: vm.as_str().to_owned(),
                    })
                    .unwrap_or(WorkloadRoute::CapabilityUnavailable {
                        provider: metadata.provider_kind,
                    }),
                WorkloadProviderKind::UnsafeLocal => WorkloadRoute::UnsafeLocal,
                provider => WorkloadRoute::CapabilityUnavailable { provider },
            };
            entries.insert(
                canonical,
                CatalogEntry {
                    metadata: metadata.clone(),
                    route,
                },
            );
        }
        Ok(Self { entries, aliases })
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }

    pub(crate) fn resolve(&self, target: &WorkloadTarget) -> Result<&CatalogEntry, CatalogError> {
        self.entries
            .get(&target.to_canonical())
            .ok_or(CatalogError::TargetNotFound)
    }

    pub(crate) fn resolve_text(&self, raw: &str) -> Result<&CatalogEntry, CatalogError> {
        if let Ok(target) = WorkloadTarget::parse(raw) {
            return self.resolve(&target);
        }
        match self.aliases.get(raw).map(Vec::as_slice) {
            Some([canonical]) => self
                .entries
                .get(canonical)
                .ok_or(CatalogError::TargetNotFound),
            Some([_, _, ..]) => Err(CatalogError::TargetAmbiguous),
            _ => Err(CatalogError::TargetNotFound),
        }
    }

    pub(crate) fn public_summary(
        entry: &CatalogEntry,
        state: WorkloadState,
        availability: WorkloadAvailability,
    ) -> WorkloadPublicSummary {
        let graphical_posture = match availability {
            WorkloadAvailability::GraphicalSessionInactive => {
                GraphicalLaunchPosture::GraphicalSessionInactive
            }
            WorkloadAvailability::WaylandUnavailable => GraphicalLaunchPosture::WaylandUnavailable,
            WorkloadAvailability::ProxyUnavailable => GraphicalLaunchPosture::ProxyUnavailable,
            _ if entry.metadata.items.iter().any(|item| item.graphical) => {
                match entry.metadata.provider_kind {
                    WorkloadProviderKind::UnsafeLocal => GraphicalLaunchPosture::Proxied,
                    _ => GraphicalLaunchPosture::NotApplicable,
                }
            }
            _ => GraphicalLaunchPosture::NotApplicable,
        };
        WorkloadPublicSummary {
            identity: entry.metadata.identity.clone(),
            provider_kind: entry.metadata.provider_kind,
            state,
            execution_posture: entry.metadata.execution_posture.clone(),
            availability,
            graphical_posture,
            capabilities: entry.metadata.capabilities.clone(),
            launcher_items: entry.metadata.items.clone(),
            default_item_id: entry.metadata.default_item_id.clone(),
        }
    }

    pub(crate) fn resolve_exec(
        &self,
        resolver: &BundleResolver,
        target: &WorkloadTarget,
        item_id: &ProtocolToken,
    ) -> Result<ResolvedExec, CatalogError> {
        let entry = self.resolve(target)?;
        let public_item = entry
            .metadata
            .items
            .iter()
            .find(|item| &item.id == item_id)
            .ok_or(CatalogError::ItemNotFound)?;
        if public_item.kind != LauncherItemKind::Exec {
            return Err(CatalogError::ConfiguredItemMismatch);
        }
        let private = resolver
            .unsafe_local_workloads
            .as_ref()
            .ok_or(CatalogError::ArtifactsUnavailable)?;
        let items = match &entry.route {
            WorkloadRoute::UnsafeLocal => private
                .workloads
                .iter()
                .find(|workload| workload.identity.canonical_target == *target)
                .map(|workload| workload.items.as_slice()),
            WorkloadRoute::LocalVm { .. } => private
                .local_vm_workloads
                .iter()
                .find(|workload| workload.identity.canonical_target == *target)
                .map(|workload| workload.items.as_slice()),
            WorkloadRoute::CapabilityUnavailable { .. } => None,
        }
        .ok_or(CatalogError::ConfiguredItemMissing)?;
        let private_item = items
            .iter()
            .find(|item| item.id() == item_id)
            .ok_or(CatalogError::ConfiguredItemMissing)?;
        let UnsafeLocalLauncherItem::Exec(private_exec) = private_item else {
            return Err(CatalogError::ConfiguredItemMismatch);
        };
        if private_exec.graphical != public_item.graphical {
            return Err(CatalogError::ConfiguredItemMismatch);
        }
        Ok(ResolvedExec {
            identity: entry.metadata.identity.clone(),
            route: entry.route.clone(),
            item_id: item_id.clone(),
            argv: private_exec.argv.clone(),
            graphical: private_exec.graphical,
        })
    }
}

fn realm_is_direct_local(resolver: &BundleResolver, identity: &WorkloadIdentity) -> bool {
    resolver
        .realm_controllers
        .as_ref()
        .is_some_and(|controllers| {
            controllers.controllers.iter().any(|controller| {
                controller.realm_path.as_str() == identity.realm_path.target_form()
                    && controller.placement == RealmControllerPlacement::HostLocal
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_ledger_is_idempotent_and_rejects_changed_fingerprint() {
        let target = WorkloadTarget::parse("browser.host.d2b").unwrap();
        let item = ProtocolToken::parse("browser").unwrap();
        let other = ProtocolToken::parse("other").unwrap();
        let operation = "launch-ledger-parity-case";
        abort_launch(65001, operation);
        assert_eq!(
            begin_launch(65001, operation, &target, &item).unwrap(),
            LaunchLedgerBegin::New
        );
        complete_launch(65001, operation);
        assert_eq!(
            begin_launch(65001, operation, &target, &item).unwrap(),
            LaunchLedgerBegin::AlreadyCommitted
        );
        assert_eq!(
            begin_launch(65001, operation, &target, &other),
            Err(CatalogError::OperationConflict)
        );
        abort_launch(65001, operation);
    }
}
