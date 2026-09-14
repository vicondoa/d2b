use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use d2b_contracts::{
    LauncherItemKind, ProtocolToken, WorkloadProviderKind, WorkloadState,
    controller_config::RealmControllerPlacement,
    launcher::LauncherWorkloadSummary,
    workload_identity::{WorkloadIdentity, WorkloadTarget},
};
use d2b_contracts_control::{
    public_wire::{GraphicalLaunchPosture, WorkloadAvailability, WorkloadPublicSummary},
    unsafe_local_wire::RealmAccentColor,
};
use d2b_core::{
    bundle_resolver::BundleResolver,
    configured_argv::ConfiguredArgv,
    unsafe_local_workloads::{
        UnsafeLocalLauncherItem, UnsafeLocalWorkloadsJson,
    },
};

use crate::typed_error::{TypedError, WorkloadLaunchErrorKind};
use crate::unsafe_local_helper::{HelperAvailability, HelperRegistryError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkloadRoute {
    LocalVm { vm: String },
    UnsafeLocal,
    CapabilityUnavailable { provider: WorkloadProviderKind },
}

/// Why a shell target could not be resolved to a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTargetError {
    TargetNotFound,
    AliasConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    ArtifactsUnavailable,
    TargetNotFound,
    LauncherDisabled,
    ItemNotFound,
    ConfiguredItemMissing,
    ConfiguredItemMismatch,
    AliasConflict,
    OperationConflict,
    OperationInProgress,
}

pub fn map_catalog_error(error: CatalogError) -> TypedError {
    let kind = match error {
        CatalogError::TargetNotFound | CatalogError::ItemNotFound | CatalogError::AliasConflict => {
            WorkloadLaunchErrorKind::ItemNotFound
        }
        CatalogError::LauncherDisabled => WorkloadLaunchErrorKind::LauncherDisabled,
        CatalogError::ArtifactsUnavailable
        | CatalogError::ConfiguredItemMissing
        | CatalogError::ConfiguredItemMismatch => WorkloadLaunchErrorKind::ConfiguredItemMismatch,
        CatalogError::OperationConflict => WorkloadLaunchErrorKind::OperationConflict,
        CatalogError::OperationInProgress => WorkloadLaunchErrorKind::QueueFull,
    };
    TypedError::WorkloadLaunchFailed { kind }
}

pub fn map_helper_registry_error(error: HelperRegistryError) -> TypedError {
    use d2b_contracts_control::unsafe_local_wire::HelperFailureCode;

    let kind = match error {
        HelperRegistryError::HelperUnavailable => WorkloadLaunchErrorKind::HelperUnavailable,
        HelperRegistryError::HelperStale | HelperRegistryError::GenerationSuperseded => {
            WorkloadLaunchErrorKind::HelperStale
        }
        HelperRegistryError::QueueFull | HelperRegistryError::OperationInProgress => {
            WorkloadLaunchErrorKind::QueueFull
        }
        HelperRegistryError::Timeout => WorkloadLaunchErrorKind::Timeout,
        HelperRegistryError::OperationIdConflict => WorkloadLaunchErrorKind::OperationConflict,
        HelperRegistryError::OperationRejected(code) => match code {
            HelperFailureCode::UserManagerUnavailable => {
                WorkloadLaunchErrorKind::UserManagerUnavailable
            }
            HelperFailureCode::GraphicalSessionInactive => {
                WorkloadLaunchErrorKind::GraphicalSessionInactive
            }
            HelperFailureCode::WaylandUnavailable => WorkloadLaunchErrorKind::WaylandUnavailable,
            HelperFailureCode::ProxyUnavailable | HelperFailureCode::FirstClientTimeout => {
                WorkloadLaunchErrorKind::ProxyUnavailable
            }
            HelperFailureCode::OperationIdConflict => WorkloadLaunchErrorKind::OperationConflict,
            HelperFailureCode::QueueFull => WorkloadLaunchErrorKind::QueueFull,
            HelperFailureCode::Timeout => WorkloadLaunchErrorKind::Timeout,
            _ => WorkloadLaunchErrorKind::Internal,
        },
        _ => WorkloadLaunchErrorKind::Internal,
    };
    TypedError::WorkloadLaunchFailed { kind }
}

pub fn workload_provider_label(provider: WorkloadProviderKind) -> &'static str {
    match provider {
        WorkloadProviderKind::LocalVm => "local-vm",
        WorkloadProviderKind::QemuMedia => "qemu-media",
        WorkloadProviderKind::ProviderManaged => "provider-managed",
        WorkloadProviderKind::UnsafeLocal => "unsafe-local",
    }
}

pub fn workload_availability_label(availability: WorkloadAvailability) -> &'static str {
    match availability {
        WorkloadAvailability::Ready => "ready",
        WorkloadAvailability::HelperUnavailable => "helper-unavailable",
        WorkloadAvailability::HelperStale => "helper-stale",
        WorkloadAvailability::UserManagerUnavailable => "user-manager-unavailable",
        WorkloadAvailability::GraphicalSessionInactive => "graphical-session-inactive",
        WorkloadAvailability::WaylandUnavailable => "wayland-unavailable",
        WorkloadAvailability::ProxyUnavailable => "proxy-unavailable",
        WorkloadAvailability::Degraded => "degraded",
    }
}

pub fn unsafe_local_workload_availability(
    helper: HelperAvailability,
    last_failure: Option<d2b_contracts_control::unsafe_local_wire::HelperFailureCode>,
) -> WorkloadAvailability {
    use d2b_contracts_control::unsafe_local_wire::HelperFailureCode;

    match helper {
        HelperAvailability::Ready => match last_failure {
            Some(HelperFailureCode::UserManagerUnavailable) => {
                WorkloadAvailability::UserManagerUnavailable
            }
            Some(HelperFailureCode::GraphicalSessionInactive) => {
                WorkloadAvailability::GraphicalSessionInactive
            }
            Some(HelperFailureCode::WaylandUnavailable) => WorkloadAvailability::WaylandUnavailable,
            Some(HelperFailureCode::ProxyUnavailable | HelperFailureCode::FirstClientTimeout) => {
                WorkloadAvailability::ProxyUnavailable
            }
            Some(_) => WorkloadAvailability::Degraded,
            None => WorkloadAvailability::Ready,
        },
        HelperAvailability::Unavailable => WorkloadAvailability::HelperUnavailable,
        HelperAvailability::Stale => WorkloadAvailability::HelperStale,
    }
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub metadata: LauncherWorkloadSummary,
    pub route: WorkloadRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchLedgerBegin {
    New,
    AlreadyCommitted,
}

#[derive(Debug, Clone)]
struct LaunchLedgerEntry {
    fingerprint: String,
    committed: bool,
    updated_at: Instant,
}

const MAX_LAUNCH_OPERATIONS_PER_UID: usize = 64;
const ACTIVE_LAUNCH_RETENTION: Duration = Duration::from_secs(45);
const COMMITTED_LAUNCH_RETENTION: Duration = Duration::from_secs(300);

fn launch_ledger() -> &'static Mutex<BTreeMap<(u32, String), LaunchLedgerEntry>> {
    static LEDGER: OnceLock<Mutex<BTreeMap<(u32, String), LaunchLedgerEntry>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn begin_launch(
    requester_uid: u32,
    operation_id: &str,
    target: &WorkloadTarget,
    item_id: &ProtocolToken,
) -> Result<LaunchLedgerBegin, CatalogError> {
    let key = (requester_uid, operation_id.to_owned());
    let fingerprint = format!("{}:{}", target.to_canonical(), item_id.as_str());
    let mut ledger = launch_ledger().lock().expect("workload launch ledger");
    let now = Instant::now();
    ledger.retain(|_, entry| {
        entry.updated_at.elapsed()
            < if entry.committed {
                COMMITTED_LAUNCH_RETENTION
            } else {
                ACTIVE_LAUNCH_RETENTION
            }
    });
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
    if ledger
        .keys()
        .filter(|(uid, _)| *uid == requester_uid)
        .count()
        >= MAX_LAUNCH_OPERATIONS_PER_UID
    {
        let oldest = ledger
            .iter()
            .filter(|((uid, _), entry)| *uid == requester_uid && entry.committed)
            .min_by_key(|(_, entry)| entry.updated_at)
            .map(|(key, _)| key.clone())
            .ok_or(CatalogError::OperationInProgress)?;
        ledger.remove(&oldest);
    }
    ledger.insert(
        key,
        LaunchLedgerEntry {
            fingerprint,
            committed: false,
            updated_at: now,
        },
    );
    Ok(LaunchLedgerBegin::New)
}

pub fn complete_launch(requester_uid: u32, operation_id: &str) {
    if let Some(entry) = launch_ledger()
        .lock()
        .expect("workload launch ledger")
        .get_mut(&(requester_uid, operation_id.to_owned()))
    {
        entry.committed = true;
        entry.updated_at = Instant::now();
    }
}

pub fn abort_launch(requester_uid: u32, operation_id: &str) {
    launch_ledger()
        .lock()
        .expect("workload launch ledger")
        .remove(&(requester_uid, operation_id.to_owned()));
}

#[derive(Debug, Clone)]
pub struct ResolvedExec {
    pub identity: WorkloadIdentity,
    pub route: WorkloadRoute,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
    pub realm_accent_color: RealmAccentColor,
}

/// How a shell target is served. The deleted unsafe-local shell route is not
/// representable here; such a target resolves to an unavailable capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellRoute {
    LocalVm { vm: String },
    CapabilityUnavailable { provider: WorkloadProviderKind },
}

#[derive(Debug, Clone)]
pub struct ResolvedShell {
    pub identity: Option<WorkloadIdentity>,
    pub route: ShellRoute,
}

#[derive(Debug, Clone)]
pub struct WorkloadCatalog {
    entries: BTreeMap<String, CatalogEntry>,
    visible: std::collections::BTreeSet<String>,
    known_local_vms: std::collections::BTreeSet<String>,
}

impl WorkloadCatalog {
    pub fn from_resolver(resolver: &BundleResolver) -> Result<Self, CatalogError> {
        let public = resolver
            .realm_workloads_launcher_v2
            .as_ref()
            .ok_or(CatalogError::ArtifactsUnavailable)?;
        let mut entries = BTreeMap::new();
        let mut visible = std::collections::BTreeSet::new();
        for metadata in &public.workloads {
            let canonical = metadata.identity.canonical_target.to_canonical();
            let direct_local = realm_is_direct_local(resolver, &metadata.identity);
            let route = if direct_local {
                route_for_provider(
                    metadata.provider_kind,
                    metadata
                        .identity
                        .legacy_vm_name
                        .as_ref()
                        .map(|vm| vm.as_str()),
                    metadata.identity.workload_id.as_str(),
                )
            } else {
                WorkloadRoute::CapabilityUnavailable {
                    provider: metadata.provider_kind,
                }
            };
            if direct_local {
                visible.insert(canonical.clone());
            }
            entries.insert(
                canonical,
                CatalogEntry {
                    metadata: metadata.clone(),
                    route,
                },
            );
        }
        Ok(Self {
            entries,
            visible,
            known_local_vms: resolver.manifest.vms.keys().cloned().collect(),
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries
            .iter()
            .filter(|(canonical, _)| self.visible.contains(*canonical))
            .map(|(_, entry)| entry)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_entries(entries: impl IntoIterator<Item = CatalogEntry>) -> Self {
        let entries = entries
            .into_iter()
            .map(|entry| {
                (
                    entry.metadata.identity.canonical_target.to_canonical(),
                    entry,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let visible = entries.keys().cloned().collect();
        let known_local_vms = entries
            .values()
            .filter_map(|entry| match &entry.route {
                WorkloadRoute::LocalVm { vm } => Some(vm.clone()),
                _ => None,
            })
            .collect();
        Self {
            entries,
            visible,
            known_local_vms,
        }
    }

    pub fn resolve(&self, target: &WorkloadTarget) -> Result<&CatalogEntry, CatalogError> {
        self.entries
            .get(&target.to_canonical())
            .ok_or(CatalogError::TargetNotFound)
    }

    pub fn public_summary(
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

    pub fn resolve_exec(
        &self,
        private: Option<&UnsafeLocalWorkloadsJson>,
        target: &WorkloadTarget,
        item_id: &ProtocolToken,
    ) -> Result<ResolvedExec, CatalogError> {
        let entry = self.resolve(target)?;
        if !entry.metadata.launcher_enabled {
            return Err(CatalogError::LauncherDisabled);
        }
        let public_item = entry
            .metadata
            .items
            .iter()
            .find(|item| &item.id == item_id)
            .ok_or(CatalogError::ItemNotFound)?;
        if public_item.kind != LauncherItemKind::Exec {
            return Err(CatalogError::ConfiguredItemMismatch);
        }
        let private = private.ok_or(CatalogError::ArtifactsUnavailable)?;
        let private_workload = match &entry.route {
            WorkloadRoute::UnsafeLocal => private
                .workloads
                .iter()
                .find(|workload| workload.identity.canonical_target == *target)
                .map(|workload| (&workload.identity, workload.items.as_slice())),
            WorkloadRoute::LocalVm { .. } => private
                .local_vm_workloads
                .iter()
                .find(|workload| workload.identity.canonical_target == *target)
                .map(|workload| (&workload.identity, workload.items.as_slice())),
            WorkloadRoute::CapabilityUnavailable { .. } => None,
        }
        .ok_or(CatalogError::ConfiguredItemMissing)?;
        if private_workload.0 != &entry.metadata.identity {
            return Err(CatalogError::ConfiguredItemMismatch);
        }
        let items = private_workload.1;
        let private_item = items
            .iter()
            .find(|item| item.id() == item_id)
            .ok_or(CatalogError::ConfiguredItemMissing)?;
        let UnsafeLocalLauncherItem::Exec(private_exec) = private_item else {
            return Err(CatalogError::ConfiguredItemMismatch);
        };
        if private_exec.name != public_item.name
            || private_exec.icon != public_item.icon
            || private_exec.graphical != public_item.graphical
        {
            return Err(CatalogError::ConfiguredItemMismatch);
        }
        Ok(ResolvedExec {
            identity: entry.metadata.identity.clone(),
            route: entry.route.clone(),
            item_id: item_id.clone(),
            argv: private_exec.argv.clone(),
            graphical: private_exec.graphical,
            realm_accent_color: RealmAccentColor::new(entry.metadata.realm_accent_color.clone())
                .map_err(|_| CatalogError::ConfiguredItemMismatch)?,
        })
    }

    pub fn resolve_shell(&self, target: &str) -> Result<ResolvedShell, ShellTargetError> {
        let Some(entry) = self.resolve_shell_entry(target)? else {
            return Ok(ResolvedShell {
                identity: None,
                route: ShellRoute::LocalVm {
                    vm: target.to_owned(),
                },
            });
        };

        Ok(ResolvedShell {
            identity: Some(entry.metadata.identity.clone()),
            route: match &entry.route {
                WorkloadRoute::LocalVm { vm } => ShellRoute::LocalVm { vm: vm.clone() },
                WorkloadRoute::CapabilityUnavailable { provider } => {
                    ShellRoute::CapabilityUnavailable {
                        provider: *provider,
                    }
                }
                // The unsafe-local shell route is deleted; a workload that only
                // declared it fails closed as an unavailable provider capability.
                WorkloadRoute::UnsafeLocal => ShellRoute::CapabilityUnavailable {
                    provider: WorkloadProviderKind::UnsafeLocal,
                },
            },
        })
    }

    fn resolve_shell_entry(&self, target: &str) -> Result<Option<&CatalogEntry>, ShellTargetError> {
        if target.ends_with(".d2b") {
            return self
                .entries
                .get(target)
                .map(Some)
                .ok_or(ShellTargetError::TargetNotFound);
        }

        if self.known_local_vms.contains(target) {
            return Ok(None);
        }

        let legacy = self
            .entries
            .values()
            .filter(|entry| matches!(&entry.route, WorkloadRoute::LocalVm { vm } if vm == target))
            .collect::<Vec<_>>();
        if let [entry] = legacy.as_slice() {
            return Ok(Some(*entry));
        }
        if legacy.len() > 1 {
            return Err(ShellTargetError::AliasConflict);
        }

        let aliases = self
            .entries
            .values()
            .filter(|entry| entry.metadata.identity.workload_id.as_str() == target)
            .collect::<Vec<_>>();
        match aliases.as_slice() {
            [] => Ok(None),
            [entry] => Ok(Some(*entry)),
            _ => Err(ShellTargetError::AliasConflict),
        }
    }
}

fn route_for_provider(
    provider: WorkloadProviderKind,
    legacy_vm_name: Option<&str>,
    workload_id: &str,
) -> WorkloadRoute {
    match provider {
        WorkloadProviderKind::LocalVm => WorkloadRoute::LocalVm {
            vm: legacy_vm_name.unwrap_or(workload_id).to_owned(),
        },
        WorkloadProviderKind::UnsafeLocal => WorkloadRoute::UnsafeLocal,
        provider => WorkloadRoute::CapabilityUnavailable { provider },
    }
}

fn realm_is_direct_local(resolver: &BundleResolver, identity: &WorkloadIdentity) -> bool {
    resolver
        .realm_controllers
        .as_ref()
        .is_some_and(|controllers| {
            controllers.controllers.iter().any(|controller| {
                controller_matches_direct_local(
                    controller.realm_path.as_str(),
                    controller.placement,
                    identity,
                )
            })
        })
}

fn controller_matches_direct_local(
    controller_realm_path: &str,
    placement: RealmControllerPlacement,
    identity: &WorkloadIdentity,
) -> bool {
    controller_realm_path == identity.realm_path.target_form()
        && placement == RealmControllerPlacement::HostLocal
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::{
        CapabilitySet, DisplayEnvironmentPosture, EnvironmentPosture, ExecutionIdentityPosture,
        IsolationPosture, LauncherIcon, LauncherItemSummary, SessionPersistencePosture,
        WorkloadExecutionPosture,
        ids::{RealmId, WorkloadId},
        realm::RealmPath,
    };
    use d2b_core::{
        configured_argv::ConfiguredArgv,
        contract_id::ContractId,
        unsafe_local_workloads::{
            LocalVmConfiguredWorkload, UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION, UnsafeLocalExecItem,
            UnsafeLocalWorkload,
            UnsafeLocalWorkloadsJson,
        },
    };

    fn workload_identity(realm: &str) -> WorkloadIdentity {
        let realm_id = RealmId::parse(realm).unwrap();
        WorkloadIdentity::new(
            WorkloadId::parse("browser").unwrap(),
            realm_id.clone(),
            RealmPath::new(vec![realm_id]).unwrap(),
            WorkloadTarget::parse(&format!("browser.{realm}.d2b")).unwrap(),
        )
    }

    fn catalog_entry(provider: WorkloadProviderKind) -> CatalogEntry {
        let mut identity = workload_identity("work");
        match provider {
            WorkloadProviderKind::LocalVm => {
                identity.legacy_vm_name = Some(ContractId::parse("corp-vm").unwrap());
                identity.runtime_kind = Some(ContractId::parse("nixos").unwrap());
                identity.provider_id = Some(ContractId::parse("local-cloud-hypervisor").unwrap());
            }
            WorkloadProviderKind::UnsafeLocal => {
                identity.runtime_kind = Some(ContractId::parse("unsafe-local").unwrap());
                identity.provider_id = Some(ContractId::parse("unsafe-local").unwrap());
            }
            _ => {}
        }
        let unsafe_local = provider == WorkloadProviderKind::UnsafeLocal;
        CatalogEntry {
            metadata: LauncherWorkloadSummary {
                identity,
                provider_kind: provider,
                execution_posture: WorkloadExecutionPosture {
                    isolation: if unsafe_local {
                        IsolationPosture::UnsafeLocal
                    } else {
                        IsolationPosture::VirtualMachine
                    },
                    environment: if unsafe_local {
                        EnvironmentPosture::SystemdUserManagerAmbient
                    } else {
                        EnvironmentPosture::RuntimeManaged
                    },
                    display_environment: if unsafe_local {
                        DisplayEnvironmentPosture::WaylandProxyOnly
                    } else {
                        DisplayEnvironmentPosture::RuntimeManaged
                    },
                    execution_identity: if unsafe_local {
                        ExecutionIdentityPosture::AuthenticatedRequesterUid
                    } else {
                        ExecutionIdentityPosture::WorkloadUser
                    },
                    session_persistence: if unsafe_local {
                        SessionPersistencePosture::UserManagerLifetime
                    } else {
                        SessionPersistencePosture::RuntimeManaged
                    },
                },
                label: "Browser".to_owned(),
                icon: LauncherIcon::default(),
                realm_accent_color: "#336699".to_owned(),
                launcher_enabled: true,
                default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
                capabilities: CapabilitySet::default(),
                items: vec![LauncherItemSummary {
                    id: ProtocolToken::parse("browser").unwrap(),
                    name: "Browser".to_owned(),
                    icon: LauncherIcon::default(),
                    kind: LauncherItemKind::Exec,
                    graphical: true,
                    capabilities: CapabilitySet::default(),
                }],
            },
            route: route_for_provider(
                provider,
                if provider == WorkloadProviderKind::LocalVm {
                    Some("corp-vm")
                } else {
                    None
                },
                "browser",
            ),
        }
    }

    fn private_artifact(entries: &[CatalogEntry]) -> UnsafeLocalWorkloadsJson {
        let exec = || {
            UnsafeLocalLauncherItem::Exec(UnsafeLocalExecItem {
                id: ProtocolToken::parse("browser").unwrap(),
                name: "Browser".to_owned(),
                icon: LauncherIcon::default(),
                argv: ConfiguredArgv::new(vec![
                    "browser-bin".to_owned(),
                    "--configured".to_owned(),
                ])
                .unwrap(),
                graphical: true,
            })
        };
        UnsafeLocalWorkloadsJson {
            schema_version: UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION.to_owned(),
            workloads: entries
                .iter()
                .filter(|entry| matches!(entry.route, WorkloadRoute::UnsafeLocal))
                .map(|entry| UnsafeLocalWorkload {
                    identity: entry.metadata.identity.clone(),
                    default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
                    items: vec![exec()],
                    shell: None,
                })
                .collect(),
            local_vm_workloads: entries
                .iter()
                .filter(|entry| matches!(entry.route, WorkloadRoute::LocalVm { .. }))
                .map(|entry| LocalVmConfiguredWorkload {
                    identity: entry.metadata.identity.clone(),
                    default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
                    items: vec![exec()],
                })
                .collect(),
        }
    }

    fn shell_entry(provider: WorkloadProviderKind, realm: &str) -> CatalogEntry {
        let mut entry = catalog_entry(provider);
        entry.metadata.identity = workload_identity(realm);
        match provider {
            WorkloadProviderKind::LocalVm => {
                entry.metadata.identity.legacy_vm_name =
                    Some(ContractId::parse("corp-vm").unwrap());
                entry.metadata.identity.runtime_kind = Some(ContractId::parse("nixos").unwrap());
                entry.metadata.identity.provider_id =
                    Some(ContractId::parse("local-cloud-hypervisor").unwrap());
            }
            WorkloadProviderKind::UnsafeLocal => {
                entry.metadata.identity.runtime_kind =
                    Some(ContractId::parse("unsafe-local").unwrap());
                entry.metadata.identity.provider_id =
                    Some(ContractId::parse("unsafe-local").unwrap());
            }
            _ => {}
        }
        entry.metadata.items.push(LauncherItemSummary {
            id: ProtocolToken::parse("terminal").unwrap(),
            name: "Terminal".to_owned(),
            icon: LauncherIcon::default(),
            kind: LauncherItemKind::Shell,
            graphical: false,
            capabilities: CapabilitySet::default(),
        });
        entry.route = route_for_provider(
            provider,
            entry
                .metadata
                .identity
                .legacy_vm_name
                .as_ref()
                .map(|value| value.as_str()),
            entry.metadata.identity.workload_id.as_str(),
        );
        entry
    }

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
        assert_eq!(
            begin_launch(65001, operation, &target, &item),
            Err(CatalogError::OperationInProgress)
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

    #[test]
    fn active_launch_capacity_isolated_per_uid() {
        let target = WorkloadTarget::parse("browser.host.d2b").unwrap();
        let item = ProtocolToken::parse("browser").unwrap();
        let saturated_uid = 65002;
        let other_uid = 65003;
        for index in 0..MAX_LAUNCH_OPERATIONS_PER_UID {
            begin_launch(saturated_uid, &format!("capacity-{index}"), &target, &item).unwrap();
        }
        assert_eq!(
            begin_launch(saturated_uid, "capacity-overflow", &target, &item),
            Err(CatalogError::OperationInProgress)
        );
        assert_eq!(
            begin_launch(other_uid, "other-user", &target, &item),
            Ok(LaunchLedgerBegin::New)
        );
        for index in 0..MAX_LAUNCH_OPERATIONS_PER_UID {
            abort_launch(saturated_uid, &format!("capacity-{index}"));
        }
        abort_launch(other_uid, "other-user");
    }

    #[test]
    fn provider_routes_never_coerce_unsafe_local_to_vm() {
        assert_eq!(
            route_for_provider(WorkloadProviderKind::UnsafeLocal, Some("host"), "browser"),
            WorkloadRoute::UnsafeLocal
        );
        assert_eq!(
            route_for_provider(WorkloadProviderKind::LocalVm, Some("corp-vm"), "browser"),
            WorkloadRoute::LocalVm {
                vm: "corp-vm".to_owned()
            }
        );
        assert_eq!(
            route_for_provider(WorkloadProviderKind::LocalVm, None, "browser"),
            WorkloadRoute::LocalVm {
                vm: "browser".to_owned()
            }
        );
    }

    #[test]
    fn only_matching_host_local_realm_is_direct() {
        let identity = workload_identity("work");
        assert!(controller_matches_direct_local(
            "work",
            RealmControllerPlacement::HostLocal,
            &identity
        ));
        assert!(!controller_matches_direct_local(
            "work",
            RealmControllerPlacement::GatewayVm,
            &identity
        ));
        assert!(!controller_matches_direct_local(
            "home",
            RealmControllerPlacement::HostLocal,
            &identity
        ));
    }

    #[test]
    fn resolve_exec_returns_only_trusted_local_vm_and_unsafe_local_descriptors() {
        for provider in [
            WorkloadProviderKind::LocalVm,
            WorkloadProviderKind::UnsafeLocal,
        ] {
            let entry = catalog_entry(provider);
            let target = entry.metadata.identity.canonical_target.clone();
            let catalog = WorkloadCatalog::from_test_entries([entry.clone()]);
            let private = private_artifact(&[entry]);
            let resolved = catalog
                .resolve_exec(
                    Some(&private),
                    &target,
                    &ProtocolToken::parse("browser").unwrap(),
                )
                .expect("matching configured descriptor resolves");
            assert_eq!(
                resolved.argv.as_slice(),
                ["browser-bin", "--configured"],
                "argv comes from the private artifact"
            );
            assert!(resolved.graphical);
            assert_eq!(resolved.realm_accent_color.as_str(), "#336699");
            assert!(
                matches!(
                    (&resolved.route, provider),
                    (WorkloadRoute::LocalVm { vm }, WorkloadProviderKind::LocalVm)
                        if vm == "corp-vm"
                ) || matches!(
                    (&resolved.route, provider),
                    (
                        WorkloadRoute::UnsafeLocal,
                        WorkloadProviderKind::UnsafeLocal
                    )
                )
            );
        }
    }

    #[test]
    fn resolve_exec_rejects_missing_mismatch_tamper_and_graphical_drift() {
        let entry = catalog_entry(WorkloadProviderKind::UnsafeLocal);
        let target = entry.metadata.identity.canonical_target.clone();
        let item = ProtocolToken::parse("browser").unwrap();

        let mut disabled = entry.clone();
        disabled.metadata.launcher_enabled = false;
        assert_eq!(
            WorkloadCatalog::from_test_entries([disabled])
                .resolve_exec(
                    Some(&private_artifact(std::slice::from_ref(&entry))),
                    &target,
                    &item,
                )
                .unwrap_err(),
            CatalogError::LauncherDisabled
        );
        assert_eq!(
            WorkloadCatalog::from_test_entries([entry.clone()])
                .resolve_exec(
                    Some(&private_artifact(std::slice::from_ref(&entry))),
                    &target,
                    &ProtocolToken::parse("missing").unwrap(),
                )
                .unwrap_err(),
            CatalogError::ItemNotFound
        );

        let mut missing = private_artifact(std::slice::from_ref(&entry));
        missing.workloads[0].items.clear();
        assert_eq!(
            WorkloadCatalog::from_test_entries([entry.clone()])
                .resolve_exec(Some(&missing), &target, &item)
                .unwrap_err(),
            CatalogError::ConfiguredItemMissing
        );

        let mut kind_mismatch = private_artifact(std::slice::from_ref(&entry));
        kind_mismatch.workloads[0].items[0] = UnsafeLocalLauncherItem::Shell(
            d2b_core::unsafe_local_workloads::UnsafeLocalShellItem {
                id: item.clone(),
                name: "Browser".to_owned(),
                icon: LauncherIcon::default(),
            },
        );
        assert_eq!(
            WorkloadCatalog::from_test_entries([entry.clone()])
                .resolve_exec(Some(&kind_mismatch), &target, &item)
                .unwrap_err(),
            CatalogError::ConfiguredItemMismatch
        );

        let mut tampered = private_artifact(std::slice::from_ref(&entry));
        tampered.workloads[0].identity.provider_id =
            Some(ContractId::parse("tampered-provider").unwrap());
        assert_eq!(
            WorkloadCatalog::from_test_entries([entry.clone()])
                .resolve_exec(Some(&tampered), &target, &item)
                .unwrap_err(),
            CatalogError::ConfiguredItemMismatch
        );

        let mut invalid_color = entry.clone();
        invalid_color.metadata.realm_accent_color = "#ABCDEF".to_owned();
        assert_eq!(
            WorkloadCatalog::from_test_entries([invalid_color])
                .resolve_exec(
                    Some(&private_artifact(std::slice::from_ref(&entry))),
                    &target,
                    &item,
                )
                .unwrap_err(),
            CatalogError::ConfiguredItemMismatch
        );

        let mut graphical_drift = private_artifact(std::slice::from_ref(&entry));
        let UnsafeLocalLauncherItem::Exec(exec) = &mut graphical_drift.workloads[0].items[0] else {
            unreachable!()
        };
        exec.graphical = false;
        assert_eq!(
            WorkloadCatalog::from_test_entries([entry])
                .resolve_exec(Some(&graphical_drift), &target, &item)
                .unwrap_err(),
            CatalogError::ConfiguredItemMismatch
        );
    }

    #[test]
    fn resolve_shell_routes_bare_and_canonical_local_vm_compatibly() {
        let legacy = shell_entry(WorkloadProviderKind::LocalVm, "work");
        let canonical = legacy.metadata.identity.canonical_target.to_canonical();
        let catalog = WorkloadCatalog::from_test_entries([legacy.clone()]);
        for target in [canonical.as_str(), "corp-vm", "browser"] {
            let resolved = catalog.resolve_shell(target).unwrap();
            assert_eq!(
                resolved.route,
                ShellRoute::LocalVm {
                    vm: "corp-vm".to_owned()
                }
            );
        }

        let mut first_class = shell_entry(WorkloadProviderKind::LocalVm, "host");
        first_class.metadata.identity.legacy_vm_name = None;
        first_class.route = route_for_provider(
            WorkloadProviderKind::LocalVm,
            None,
            first_class.metadata.identity.workload_id.as_str(),
        );
        let canonical = first_class
            .metadata
            .identity
            .canonical_target
            .to_canonical();
        let catalog = WorkloadCatalog::from_test_entries([first_class]);
        assert_eq!(
            catalog.resolve_shell(&canonical).unwrap().route,
            ShellRoute::LocalVm {
                vm: "browser".to_owned()
            }
        );

        assert_eq!(
            WorkloadCatalog::from_test_entries([])
                .resolve_shell("legacy-vm")
                .unwrap()
                .route,
            ShellRoute::LocalVm {
                vm: "legacy-vm".to_owned()
            }
        );
    }

    #[test]
    fn resolve_shell_fails_closed_for_the_deleted_unsafe_local_route() {
        let entry = shell_entry(WorkloadProviderKind::UnsafeLocal, "host");
        let canonical = entry.metadata.identity.canonical_target.to_canonical();
        let catalog = WorkloadCatalog::from_test_entries([entry]);
        for target in [canonical.as_str(), "browser"] {
            let resolved = catalog.resolve_shell(target).unwrap();
            assert_eq!(
                resolved.route,
                ShellRoute::CapabilityUnavailable {
                    provider: WorkloadProviderKind::UnsafeLocal
                }
            );
            assert!(resolved.identity.is_some());
        }
    }

    #[test]
    fn known_bare_vm_name_precedes_unsafe_local_short_alias() {
        let entry = shell_entry(WorkloadProviderKind::UnsafeLocal, "host");
        let mut catalog = WorkloadCatalog::from_test_entries([entry]);
        catalog.known_local_vms.insert("browser".to_owned());
        let resolved = catalog.resolve_shell("browser").unwrap();
        assert_eq!(
            resolved.route,
            ShellRoute::LocalVm {
                vm: "browser".to_owned()
            }
        );
        assert!(resolved.identity.is_none());
    }

    #[test]
    fn resolve_shell_rejects_unsupported_remote_and_ambiguous_routes() {
        let mut unsupported = shell_entry(WorkloadProviderKind::QemuMedia, "media");
        unsupported.route = WorkloadRoute::CapabilityUnavailable {
            provider: WorkloadProviderKind::QemuMedia,
        };
        let canonical = unsupported
            .metadata
            .identity
            .canonical_target
            .to_canonical();
        let catalog = WorkloadCatalog::from_test_entries([unsupported]);
        assert_eq!(
            catalog.resolve_shell(&canonical).unwrap().route,
            ShellRoute::CapabilityUnavailable {
                provider: WorkloadProviderKind::QemuMedia
            }
        );

        let first = shell_entry(WorkloadProviderKind::UnsafeLocal, "work");
        let second = shell_entry(WorkloadProviderKind::UnsafeLocal, "personal");
        let catalog = WorkloadCatalog::from_test_entries([first, second]);
        assert_eq!(
            catalog.resolve_shell("browser").unwrap_err(),
            ShellTargetError::AliasConflict
        );
        assert_eq!(
            catalog.resolve_shell("missing.host.d2b").unwrap_err(),
            ShellTargetError::TargetNotFound
        );
    }
}
