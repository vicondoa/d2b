//! Daemon startup storage/restart/synchronization contract checks.
//!
//! ADR 0034 makes storage, process restart, and lock ownership explicit
//! generated contracts. This module is the daemon-side startup gate that
//! checks those contracts before any broad cleanup or adoption logic grows
//! around them. It is intentionally read-only: it never mutates storage,
//! never opens lock files, and never treats persisted PID values as
//! authority.

use std::collections::BTreeSet;

use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::storage::RestartClass;
pub use d2b_core::storage_lifecycle::{
    StorageContractValidationReason, StorageLifecycleIssue, StorageLifecycleReport,
    SyncContractValidationReason, classify_storage_validation_reason,
    classify_sync_validation_reason, storage_validation_offending_id, sync_validation_offending_id,
};

pub const STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION: u32 = 6;
pub const STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION: &str = "v2";

pub fn bundle_resolver_unavailable_report() -> StorageLifecycleReport {
    StorageLifecycleReport {
        schema_version: STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION.to_owned(),
        storage_contract_present: false,
        sync_contract_present: false,
        path_count: 0,
        restart_policy_count: 0,
        lock_count: 0,
        issues: vec![StorageLifecycleIssue::BundleResolverUnavailable],
    }
}

/// Run a read-only startup check over the bundle's storage/restart/sync
/// contracts. Output is bounded to ids and closed reason kinds; it never
/// includes raw storage paths.
pub fn run_startup_contract_check(resolver: &BundleResolver) -> StorageLifecycleReport {
    let mut issues = Vec::new();

    let storage = resolver.storage.as_ref();
    let sync = resolver.sync.as_ref();
    let contracts_expected =
        resolver.bundle.bundle_version >= STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION;

    if let Some(storage) = storage {
        if let Err(detail) = storage.validate_unique_ids() {
            issues.push(StorageLifecycleIssue::StorageContractInvalid {
                contract_id: "storage.json".to_owned(),
                reason: classify_storage_validation_reason(&detail),
                offending_id: storage_validation_offending_id(&detail),
            });
        }
    } else if contracts_expected {
        issues.push(StorageLifecycleIssue::MissingStorageContract);
    } else {
        issues.push(StorageLifecycleIssue::LegacyBundleContractsUnavailable {
            bundle_version: resolver.bundle.bundle_version,
        });
    }

    if let Some(sync) = sync {
        if let Err(detail) = sync.validate_lock_order() {
            issues.push(StorageLifecycleIssue::SyncContractInvalid {
                contract_id: "sync.json".to_owned(),
                reason: classify_sync_validation_reason(&detail),
                offending_id: sync_validation_offending_id(&detail),
            });
        }
    } else if contracts_expected {
        issues.push(StorageLifecycleIssue::MissingSyncContract);
    } else {
        if !issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::LegacyBundleContractsUnavailable { .. }
            )
        }) {
            issues.push(StorageLifecycleIssue::LegacyBundleContractsUnavailable {
                bundle_version: resolver.bundle.bundle_version,
            });
        }
    }

    if let Some(storage) = storage {
        let restart_keys: BTreeSet<(&str, &str)> = storage
            .restart_policies
            .iter()
            .map(|policy| (policy.vm.as_str(), policy.role_id.as_str()))
            .collect();
        for dag in &resolver.processes.vms {
            for node in &dag.nodes {
                let key = (dag.vm.as_str(), node.id.0.as_str());
                if !restart_keys.contains(&key) {
                    issues.push(StorageLifecycleIssue::MissingRestartPolicy {
                        vm: dag.vm.clone(),
                        role_id: node.id.0.clone(),
                    });
                }
            }
        }
        for policy in &storage.restart_policies {
            if policy.restart_class == RestartClass::Adoptable
                && policy.adoption_inputs.cgroup_leaf.is_none()
            {
                issues.push(StorageLifecycleIssue::AdoptableMissingCgroupLeaf {
                    vm: policy.vm.as_str().to_owned(),
                    role_id: policy.role_id.as_str().to_owned(),
                });
            }
        }
    }

    StorageLifecycleReport {
        schema_version: STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION.to_owned(),
        storage_contract_present: storage.is_some(),
        sync_contract_present: sync.is_some(),
        path_count: storage.map(|s| s.paths.len()).unwrap_or_default(),
        restart_policy_count: storage
            .map(|s| s.restart_policies.len())
            .unwrap_or_default(),
        lock_count: sync.map(|s| s.locks.len()).unwrap_or_default(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use d2b_core::bundle::Bundle;
    use d2b_core::bundle_resolver::BundleResolver;
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::{
        ProcessNode, ProcessRole, ProcessesJson, RoleProfile, VmProcessDag, VmProcessInvariants,
    };
    use d2b_core::storage::{
        AdoptionInputs, CleanupPolicy, DegradeScope, DegradedReason, LedgerStorageClass,
        ProcessRestartPolicy, ReadinessAfterAdopt, ReadinessAfterAdoptKind, RemediationSpec,
        RepairPolicy, RestartClass, StorageAdoptionPolicy, StorageJson, StorageLifecycle,
        StoragePathKind, StoragePathSpec, StoragePersistence, StorageRestartPolicy,
    };
    use d2b_core::sync::{
        FdPassingMechanism, FdPassingPolicy, InheritancePolicy, LockAcquireOrder,
        LockAdoptionPolicy, LockKind, LockScopeClass, LockSpec, LockStaleKind, LockStalePolicy,
        LockTimeoutKind, LockTimeoutPolicy, SyncJson,
    };
    use d2b_core::{
        contract_id::{ContractId, ContractText, PathTemplate},
        minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet},
        storage::{
            ActorKind, ActorRef, LeaseClass, PrincipalKind, PrincipalRef, SensitivityClass,
            StorageInvariant,
        },
    };

    use super::*;

    #[test]
    fn reports_clean_when_every_process_has_restart_policy_and_lock_contracts() {
        let resolver = resolver_with_contracts(true, true);
        let report = run_startup_contract_check(&resolver);
        assert!(!report.is_degraded(), "{report:?}");
        assert_eq!(report.restart_policy_count, 1);
        assert_eq!(report.lock_count, 1);
    }

    #[test]
    fn reports_missing_restart_policy() {
        let resolver = resolver_with_contracts(false, true);
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::MissingRestartPolicy { vm, role_id }
                    if vm == "corp-vm" && role_id == "cloud-hypervisor"
            )
        }));
    }

    #[test]
    fn reports_missing_restart_policy_for_each_process_node() {
        let mut resolver = resolver_with_contracts(false, true);
        let mut second = resolver.processes.vms[0].nodes[0].clone();
        second.id = d2b_core::processes::NodeId("virtiofsd-ro-store".to_owned());
        second.role = ProcessRole::Virtiofsd;
        second.profile.profile_id = "vm-corp-vm-virtiofsd-ro-store".to_owned();
        second.profile.cgroup_placement.subtree = "d2b.slice/corp-vm/virtiofsd-ro-store".to_owned();
        resolver.processes.vms[0].nodes.push(second);

        let report = run_startup_contract_check(&resolver);
        let missing: BTreeSet<_> = report
            .issues
            .iter()
            .filter_map(|issue| match issue {
                StorageLifecycleIssue::MissingRestartPolicy { vm, role_id } => {
                    Some((vm.as_str(), role_id.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            missing,
            BTreeSet::from([
                ("corp-vm", "cloud-hypervisor"),
                ("corp-vm", "virtiofsd-ro-store")
            ])
        );
    }

    #[test]
    fn reports_adoptable_policy_without_cgroup_leaf() {
        let resolver = resolver_with_contracts(true, false);
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::AdoptableMissingCgroupLeaf { vm, role_id }
                    if vm == "corp-vm" && role_id == "cloud-hypervisor"
            )
        }));
    }

    #[test]
    fn reports_missing_contracts_for_current_bundle() {
        let resolver =
            resolver_with_optional_contracts(STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION, None, None);
        let report = run_startup_contract_check(&resolver);
        assert!(
            report
                .issues
                .contains(&StorageLifecycleIssue::MissingStorageContract)
        );
        assert!(
            report
                .issues
                .contains(&StorageLifecycleIssue::MissingSyncContract)
        );
    }

    #[test]
    fn reports_legacy_bundle_without_warning_issue_fanout() {
        let resolver = resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION - 1,
            None,
            None,
        );
        let report = run_startup_contract_check(&resolver);
        assert!(report.has_only_legacy_contract_issue(), "{report:?}");
    }

    #[test]
    fn reports_invalid_storage_contract_without_raw_detail() {
        let mut storage = storage(true, true);
        storage.paths.push(storage.paths[0].clone());
        let resolver = resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION,
            Some(storage),
            Some(sync()),
        );
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::StorageContractInvalid {
                    contract_id,
                    reason: StorageContractValidationReason::DuplicateStoragePathId,
                    offending_id
                } if contract_id == "storage.json" && offending_id.as_deref() == Some("path:run-root")
            )
        }));
        let serialized = serde_json::to_string(&report).expect("serialize report");
        assert!(serialized.contains("duplicate-storage-path-id"));
        assert!(serialized.contains("path:run-root"));
        assert!(!serialized.contains("/run/d2b"));
    }

    #[test]
    fn reports_invalid_sync_contract_without_raw_detail() {
        let mut sync = sync();
        sync.locks.push(sync.locks[0].clone());
        let resolver = resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION,
            Some(storage(true, true)),
            Some(sync),
        );
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::SyncContractInvalid {
                    contract_id,
                    reason: SyncContractValidationReason::DuplicateLockId,
                    offending_id
                } if contract_id == "sync.json" && offending_id.as_deref() == Some("lock:daemon")
            )
        }));
    }

    #[test]
    fn reports_sync_ofd_and_fd_transfer_policy_as_reason_slugs() {
        let mut missing_cloexec = sync();
        missing_cloexec.locks[0].cloexec_required = false;
        let resolver = resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION,
            Some(storage(true, true)),
            Some(missing_cloexec),
        );
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::SyncContractInvalid {
                    contract_id,
                    reason: SyncContractValidationReason::OfdLockMissingCloexec,
                    offending_id
                } if contract_id == "sync.json" && offending_id.as_deref() == Some("lock:daemon")
            )
        }));
        let serialized = serde_json::to_string(&report).expect("serialize OFD report");
        assert!(serialized.contains("ofd-lock-missing-cloexec"));
        assert!(serialized.contains("lock:daemon"));

        let mut missing_lease = sync();
        missing_lease.locks[0].fd_passing_policy.mechanism = FdPassingMechanism::ScmRights;
        missing_lease.locks[0]
            .fd_passing_policy
            .lease_transfer_record_required = false;
        let resolver = resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION,
            Some(storage(true, true)),
            Some(missing_lease),
        );
        let report = run_startup_contract_check(&resolver);
        assert!(report.issues.iter().any(|issue| {
            matches!(
                issue,
                StorageLifecycleIssue::SyncContractInvalid {
                    contract_id,
                    reason: SyncContractValidationReason::FdPassingMissingLeaseTransferRecord,
                    offending_id
                } if contract_id == "sync.json" && offending_id.as_deref() == Some("lock:daemon")
            )
        }));
        let serialized = serde_json::to_string(&report).expect("serialize fd report");
        assert!(serialized.contains("fd-passing-missing-lease-transfer-record"));
        assert!(serialized.contains("lock:daemon"));
    }

    #[test]
    fn startup_report_carries_diagnostics_without_pidfd_authority() {
        let resolver = resolver_with_contracts(true, false);
        let report = run_startup_contract_check(&resolver);
        let serialized = serde_json::to_string(&report).expect("serialize report");
        assert!(serialized.contains("adoptable-missing-cgroup-leaf"));
        assert!(!serialized.contains("pidfd"));
        assert!(!serialized.contains("pidFd"));
        assert!(!serialized.contains("state.json"));
    }

    #[test]
    fn classifiers_fall_back_to_unclassified_for_unknown_details() {
        assert_eq!(
            classify_storage_validation_reason("future storage validation failure"),
            StorageContractValidationReason::Unclassified,
        );
        assert_eq!(
            classify_sync_validation_reason("future sync validation failure"),
            SyncContractValidationReason::Unclassified,
        );
        assert_eq!(
            storage_validation_offending_id("duplicate storage path id path:run-root").as_deref(),
            Some("path:run-root"),
        );
        assert_eq!(
            sync_validation_offending_id("lock lock:two shares acquire order key with lock:one")
                .as_deref(),
            Some("lock:two"),
        );
        assert_eq!(
            storage_validation_offending_id("duplicate storage path id /run/d2b").as_deref(),
            None,
        );
    }

    #[test]
    fn bundle_resolver_unavailable_report_is_degraded_and_schema_current() {
        let report = bundle_resolver_unavailable_report();
        assert_eq!(
            report.schema_version,
            STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION
        );
        assert!(report.is_degraded());
        assert_eq!(
            report.issues,
            vec![StorageLifecycleIssue::BundleResolverUnavailable]
        );
    }

    fn resolver_with_contracts(include_restart: bool, include_cgroup_leaf: bool) -> BundleResolver {
        resolver_with_optional_contracts(
            STORAGE_LIFECYCLE_CONTRACT_BUNDLE_VERSION,
            Some(storage(include_restart, include_cgroup_leaf)),
            Some(sync()),
        )
    }

    fn resolver_with_optional_contracts(
        bundle_version: u32,
        storage_contract: Option<StorageJson>,
        sync_contract: Option<SyncJson>,
    ) -> BundleResolver {
        let bundle = Bundle {
            bundle_version,
            schema_version: "v2".to_owned(),
            public_manifest_path: "manifest.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: storage_contract.as_ref().map(|_| "storage.json".to_owned()),
            sync_path: sync_contract.as_ref().map(|_| "sync.json".to_owned()),
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            unsafe_local_workloads_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: d2b_core::bundle::BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        BundleResolver::from_artifacts_with_optional_contracts(
            bundle,
            minimal_host(),
            processes(),
            storage_contract,
            sync_contract,
            None,
            None,
            manifest(),
        )
    }

    fn actor(kind: ActorKind, value: &str) -> ActorRef {
        ActorRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn principal(kind: PrincipalKind, value: &str) -> PrincipalRef {
        PrincipalRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn storage(include_restart: bool, include_cgroup_leaf: bool) -> StorageJson {
        let restart_policies = if include_restart {
            vec![ProcessRestartPolicy {
                vm: ContractId::parse("corp-vm").unwrap(),
                role_id: ContractId::parse("cloud-hypervisor").unwrap(),
                restart_class: RestartClass::Adoptable,
                adoption_inputs: AdoptionInputs {
                    cgroup_leaf: include_cgroup_leaf
                        .then(|| ContractId::parse("d2b.slice/corp-vm/cloud-hypervisor").unwrap()),
                    identity_checks: Vec::new(),
                },
                persistent_state_refs: Vec::new(),
                runtime_state_refs: Vec::new(),
                cleanup_before_restart: false,
                degrade_on_failure: DegradedReason::AdoptionQuarantined,
                degrade_scope: DegradeScope::Role,
                readiness_after_adopt: ReadinessAfterAdopt {
                    kind: ReadinessAfterAdoptKind::ExistingPredicate,
                    storage_ref: None,
                },
                remediation_id: ContractId::parse("remediate:vm-status").unwrap(),
            }]
        } else {
            Vec::new()
        };
        StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![StoragePathSpec {
                id: ContractId::parse("path:run-root").unwrap(),
                scope: ContractId::parse("host").unwrap(),
                path_template: PathTemplate::parse("/run/d2b").unwrap(),
                kind: StoragePathKind::Directory,
                lifecycle: StorageLifecycle::BootScopedReadoptable,
                persistence: StoragePersistence::BootScoped,
                owner: principal(PrincipalKind::User, "d2bd"),
                group: principal(PrincipalKind::Group, "d2b"),
                mode: "0750".to_owned(),
                access_acl: Vec::new(),
                default_acl: Vec::new(),
                creator: actor(ActorKind::Daemon, "d2bd"),
                writers: vec![actor(ActorKind::Daemon, "d2bd")],
                readers: vec![actor(ActorKind::Daemon, "d2bd")],
                cleanup_policy: CleanupPolicy::Boot,
                repair_policy: RepairPolicy::BrokerReconcile,
                restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
                adoption_policy: StorageAdoptionPolicy::AdoptWithLiveOwnerProof,
                lease_class: LeaseClass::None,
                sensitivity: SensitivityClass::Private,
                no_follow: true,
                recursive: false,
                invariants: vec![StorageInvariant::NoSymlink],
            }],
            restart_policies,
            degraded_states: vec![d2b_core::storage::DegradedStateSpec {
                reason: DegradedReason::AdoptionQuarantined,
                scope: DegradeScope::Role,
                storage_class: LedgerStorageClass::TamperEvidentSegmented,
                remediation_id: ContractId::parse("remediate:vm-status").unwrap(),
            }],
            remediations: vec![RemediationSpec {
                id: ContractId::parse("remediate:vm-status").unwrap(),
                command: ContractText::parse("d2b vm status <vm>").unwrap(),
                description: ContractText::parse("Inspect VM status").unwrap(),
            }],
        }
    }

    fn sync() -> SyncJson {
        SyncJson {
            schema_version: "v2".to_owned(),
            locks: vec![LockSpec {
                id: ContractId::parse("lock:daemon").unwrap(),
                scope: ContractId::parse("host").unwrap(),
                path_template: Some(PathTemplate::parse("/run/d2b/daemon.lock").unwrap()),
                resource_id: None,
                kind: LockKind::Ofd,
                owner_process: actor(ActorKind::Daemon, "d2bd"),
                allowed_holders: vec![actor(ActorKind::Daemon, "d2bd")],
                inheritance_policy: InheritancePolicy::CloseOnExec,
                fd_passing_policy: FdPassingPolicy {
                    mechanism: FdPassingMechanism::None,
                    lease_transfer_record_required: false,
                },
                acquire_order: LockAcquireOrder {
                    scope_class: LockScopeClass::Global,
                    anchored_root: ContractId::parse("run").unwrap(),
                    normalized_path: ContractId::parse("daemon.lock").unwrap(),
                    lock_id: ContractId::parse("lock:daemon").unwrap(),
                },
                timeout_policy: LockTimeoutPolicy {
                    kind: LockTimeoutKind::FailFast,
                    timeout_ms: None,
                },
                stale_policy: LockStalePolicy {
                    kind: LockStaleKind::PidfdProofRequired,
                    degraded_reason: DegradedReason::LockOwnerAmbiguous,
                },
                adoption_policy: LockAdoptionPolicy::ReacquireAfterProof,
                degrade_scope: DegradeScope::Host,
                release_authority: actor(ActorKind::Daemon, "d2bd"),
                cloexec_required: true,
            }],
        }
    }

    fn processes() -> ProcessesJson {
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![VmProcessDag {
                workload_identity: None,
                vm: "corp-vm".to_owned(),
                nodes: vec![ProcessNode {
                    id: d2b_core::processes::NodeId("cloud-hypervisor".to_owned()),
                    role: ProcessRole::CloudHypervisorRunner,
                    unit: None,
                    binary_path: Some("/nix/store/ch/bin/cloud-hypervisor".to_owned()),
                    argv: vec!["cloud-hypervisor".to_owned()],
                    env: Vec::new(),
                    plan_ops: Vec::new(),
                    network_interfaces: Vec::new(),
                    profile: RoleProfile {
                        profile_id: "vm-corp-vm-cloud-hypervisor".to_owned(),
                        uid: 1000,
                        gid: 1000,
                        adr_carve_out: None,
                        caps: Vec::new(),
                        namespaces: NamespaceSet {
                            mount: true,
                            pid: false,
                            net: false,
                            ipc: true,
                            uts: false,
                            user: false,
                        },
                        seccomp_policy_ref: Some("w1-cloud-hypervisor-runner".to_owned()),
                        mount_policy: MountPolicy {
                            read_only_paths: Vec::new(),
                            writable_paths: Vec::new(),
                            nix_store_read_only: true,
                            hide_device_nodes_by_default: true,
                            device_binds: Vec::new(),
                            bind_mounts: Vec::new(),
                        },
                        cgroup_placement: CgroupPlacement {
                            subtree: "d2b.slice/corp-vm/cloud-hypervisor".to_owned(),
                            controllers: Vec::new(),
                            delegated: false,
                        },
                        user_namespace: None,
                        umask: None,
                    },
                    readiness: Vec::new(),
                }],
                edges: Vec::new(),
                invariants: VmProcessInvariants {
                    swtpm_pre_start_flush: true,
                    per_vm_audit_pipeline: true,
                    usbip_gating: true,
                    tpm_ownership_migration_without_running_vm_mutation: true,
                },
            }],
        }
    }

    fn minimal_host() -> HostJson {
        serde_json::from_str(r##"{
            "schemaVersion":"v2",
            "site":{"allowUnsafeEastWest":false},
            "environments":[],
            "nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"test"},
            "hostsFile":{"startMarker":"# begin","endMarker":"# end","rule":"test"},
            "networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b.conf","matchCriteria":[],"reloadBehavior":"none","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace-managed-block"}},
            "kernelModules":[],
            "fdOwnership":[],
            "cloudHypervisorCapabilities":[],
            "ifNameMappings":[],
            "ch":{"netHandoffMode":"tap-fd"},
            "firewallCoexistencePolicy":{"manager":"none","policy":"coexist","rationale":"test"}
        }"##)
        .expect("minimal HostJson")
    }

    fn manifest() -> ManifestV04 {
        ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"obsVsockCid":0,"obsVsockHostSocket":"","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"signozUrl":"","vmName":""},"corp-vm":{"apiSocket":"/run/d2b/vms/corp-vm/api.sock","audio":false,"audioService":"","audioStateFile":"","bridge":"d2b-brTEST","env":"work","mtu":null,"mssClamp":null,"lan":null,"gpuSocket":"","graphics":false,"isNetVm":false,"name":"corp-vm","netVm":null,"observability":{"agentSocket":"","enabled":false,"vsockCid":0,"vsockHostSocket":""},"runtime":{"kind":"nixos","provider":{"id":"local-cloud-hypervisor","type":"local","driver":"cloud-hypervisor"},"capabilities":{"lifecycle":true,"display":true,"usbHotplug":true,"guestControl":true,"exec":true,"configSync":true,"ssh":true,"storeSync":true,"keys":true,"inGuestObservability":true}},"sshUser":"alice","stateDir":"/var/lib/d2b/vms/corp-vm","staticIp":"10.20.0.10","tap":"d2b-tTEST","tpm":false,"tpmSocket":"","usbipYubikey":false,"usbipdHostIp":null}}"#,
        ).expect("minimal ManifestV04")
    }
}
