//! The broker's own authority for the private execution values an admitted
//! effect resolves to (U10, KTD8).
//!
//! [`BundleExecutionValues`] is the broker's production answer to
//! [`d2b_core::execution_plan::PrivateExecutionValues`]. Every value it
//! returns is read from an artifact the broker already loads under its own
//! ownership and hash checks - the private bundle, the per-Zone resource
//! bundle inside it, and the trusted storage contract beside it - or from the
//! broker's own serve-time runtime root.
//!
//! Three properties are the whole of this module, and each is why it is not a
//! passthrough:
//!
//! 1. **Every method is a lookup.** Each one is keyed by an identity the
//!    broker already holds: an accepted relationship key, a committed
//!    `Operation`. There is no parameter anywhere in this struct that could
//!    carry a program, an argument, an environment entry, a uid, a gid, or a
//!    mount policy, so an implementation physically cannot hand one back to
//!    the plan even if a caller asked for one.
//! 2. **Absence is a refusal.** Every `None` below is a relationship, a
//!    volume, a template, or a runner the verified bundle does not name. The
//!    table entry is simply absent, and
//!    [`d2b_core::execution_plan::resolve_execution_plan`] refuses the effect
//!    by name. Nothing defaults to a path, a uid, or a program.
//! 3. **Ambiguity is a refusal.** A template the bundle declares for more
//!    than one execution resolves to nothing, because the admitted carrier
//!    names the operation and the relationship and not which execution it
//!    runs for. Picking one would be the guess this module exists to avoid.

use std::path::PathBuf;
use std::sync::Arc;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{SourceKind, VolumeSpec};
use d2b_contracts_resource::v3::{
    BindingKey, BindingKind, BindingRealizationFacet, CallableOperation, FreshnessTuple,
    OperationImplementation, RequestedRights, ResourceRef, ZoneId,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::execution_plan::{
    PlannedDestination, PlannedExecutable, PlannedIdentity, PlannedView, PrivateBacking,
    PrivateExecutionValues, PrivatePath, PrivateSourceValues,
};

/// The verified private artifacts plus the broker's own runtime root.
///
/// `committed` is the committed row state the broker can independently
/// observe, and it is an input rather than something this module derives: a
/// row's desired revision and digest are the manager's facts about its own
/// desired store, and the broker's only record of them is the authority
/// projection that accepted them. An empty set is an honest absence, and it
/// refuses every effect rather than admitting one against an unfenced
/// dependency, which is why it is passed in rather than computed here.
pub(crate) struct BundleExecutionValues {
    resolver: Arc<BundleResolver>,
    runtime_root: PathBuf,
    committed: Vec<FreshnessTuple>,
}

impl BundleExecutionValues {
    /// Bind the verified bundle, the broker's own runtime root, and the
    /// committed row state the broker observes.
    pub(crate) const fn new(
        resolver: Arc<BundleResolver>,
        runtime_root: PathBuf,
        committed: Vec<FreshnessTuple>,
    ) -> Self {
        Self {
            resolver,
            runtime_root,
            committed,
        }
    }


    /// The declared `Volume` row one Zone's verified resource bundle carries.
    fn volume_spec(&self, zone: &ZoneId, volume: &ResourceRef) -> Option<VolumeSpec> {
        let row = self.resolver.find_volume_resource(zone, volume.name().as_str())?;
        serde_json::from_slice::<VolumeSpec>(row.spec().to_canonical_bytes().as_slice()).ok()
    }

    /// The trusted storage path row one `Volume` source policy selects.
    ///
    /// The two published aliases of the shared state root map onto the one row
    /// that declares it. That is a mapping between two declared row names
    /// rather than a default path, and a source policy that names neither
    /// alias nor a `path:` row resolves to nothing.
    fn storage_path_id(spec: &VolumeSpec) -> Option<String> {
        let policy = spec.source().settings().source_policy_id()?.as_str();
        Some(if matches!(policy, "state-root" | "default-state") {
            "path:state-root".to_owned()
        } else {
            format!("path:{policy}")
        })
    }

    /// The private mount point one consumer slot presents its sources in.
    ///
    /// The destination is derived from the relationship's own key - its Zone,
    /// its consumer, and its stable slot - under the broker's own runtime
    /// root. The caller names the relationship; the broker decides where it
    /// lands, which is exactly what removes arbitrary mount policy from the
    /// boundary (R37, AE7).
    fn mount_point(&self, key: &BindingKey) -> Option<PrivatePath> {
        let path = self
            .runtime_root
            .join("effects")
            .join(key.zone().as_str())
            .join(key.consumer_ref().name().as_str())
            .join(key.slot().as_str());
        PrivatePath::parse(path.to_string_lossy().into_owned()).ok()
    }
}

impl PrivateExecutionValues for BundleExecutionValues {
    fn observed(&self) -> &[FreshnessTuple] {
        &self.committed
    }

    /// The private host values one accepted `Volume` relationship resolves
    /// to, read from the verified Zone resource bundle.
    ///
    /// The backing class is the `Volume` row's own declared source kind, so a
    /// `Device` source can never resolve to a pathname this module chose. The
    /// views are the row's own declared view names and relative paths, each
    /// resolved through the trusted storage contract, and each presented under
    /// observation only: a view is a narrower claim than its relationship,
    /// and observation is the narrowest claim that still mounts.
    fn source(&self, key: &BindingKey) -> Option<PrivateSourceValues> {
        if key.kind() != BindingKind::Volume {
            // A relationship family whose private host values this resolver
            // does not derive from the verified bundle has no entry here.
            return None;
        }
        let spec = self.volume_spec(key.zone(), key.source_ref())?;
        if !matches!(
            spec.source().settings().kind(),
            SourceKind::LocalPath | SourceKind::NixClosure
        ) {
            // A block image, a tmpfs, and every source kind the verified row
            // does not declare as a host presentation this resolver realizes.
            return None;
        }
        let storage_path_id = Self::storage_path_id(&spec)?;
        let volume = key.source_ref().name().as_str();
        let root = self
            .resolver
            .resolve_volume_view_root(&storage_path_id, volume, "")?;
        let backing_path = PrivatePath::parse(root.to_string_lossy().into_owned()).ok()?;
        let mut views = Vec::with_capacity(spec.views().len());
        for (name, view) in spec.views() {
            let Ok(name) = BoundedToken::parse(name.clone()) else {
                continue;
            };
            let Some(path) =
                self.resolver
                    .resolve_volume_view_root(&storage_path_id, volume, view.path())
            else {
                continue;
            };
            let Ok(path) = PrivatePath::parse(path.to_string_lossy().into_owned()) else {
                continue;
            };
            views.push(PlannedView::new(name, RequestedRights::Observe, path));
        }
        PrivateSourceValues::new(PrivateBacking::Filesystem, backing_path, views).ok()
    }

    fn destination(&self, key: &BindingKey) -> Option<PlannedDestination> {
        Some(PlannedDestination::new(
            key.address(),
            BindingRealizationFacet::FilesystemPresentation,
            self.mount_point(key)?,
            true,
        ))
    }

    /// The identity one admitted subject resolves to.
    ///
    /// The verified bundle's runner intents are keyed by execution target,
    /// VM, and role; a bare subject reference is none of those, and the
    /// admitted carrier carries no execution target. There is exactly one
    /// honest answer here, and it is no answer: this refuses rather than
    /// picking the first intent the bundle happens to hold. An invocation
    /// whose subject resolves no identity runs without one, which the plan
    /// records and the launch posture refuses.
    fn identity(&self, _subject: &ResourceRef) -> Option<PlannedIdentity> {
        None
    }

    /// The trusted executable one committed `Operation` resolves to.
    ///
    /// The program, the arguments, and the environment are the verified
    /// bundle's own trusted runner intent for the template the committed
    /// contract names - never the request's. A template the bundle does not
    /// carry, or carries for more than one execution, resolves to nothing
    /// rather than to an empty argv or an inherited environment.
    fn executable(
        &self,
        _operation: &ResourceRef,
        declared: &CallableOperation,
    ) -> Option<PlannedExecutable> {
        let OperationImplementation::TrustedExecutableTemplate { template, .. } =
            declared.implementation()
        else {
            return None;
        };
        let intent = self
            .resolver
            .find_unique_runner_intent_for_template(template.as_str())?;
        let program = PrivatePath::parse(intent.binary_path.to_string_lossy().into_owned()).ok()?;
        // argv[0] is the program the bundle names, not a string the caller
        // supplied, so the two cannot disagree.
        let mut argv = Vec::with_capacity(intent.argv.len().max(1));
        argv.push(program.as_path().display().to_string());
        argv.extend(intent.argv.iter().skip(1).cloned());
        PlannedExecutable::new(
            declared.implementation().clone(),
            template.clone(),
            program,
            argv,
            intent.env.clone(),
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{
        AuditMode, AuthoritySubject, AuthoritySubjectKind, BindingArbitration, BindingKey,
        BindingKind, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
        BrokerRequirement, CallableOperation, CanonicalJsonObject, DesiredDigest, DesiredRevision,
        FdContract, FdKind, FreshnessTuple, OperationAudit, OperationAuthority, OperationBounds,
        OperationDomain, OperationFds, OperationImplementation, OperationSurface, PayloadProvenance,
        PayloadSchema, RequestedRights, ResourceName, ResourceRef, ResourceTypeName, ResourceUid,
        SecretAccess, SourceAdmission, StoreIncarnation,
    };
    use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken};
    use d2b_contracts_zone_session::v3::resource_bundle::{
        BundleResource, BundleResourceMetadata, ResourceBundle,
    };
    use d2b_contracts_zone_session::v3::role::{AuthorizedRole, RoleResourceVerb, RoleRule};
    use d2b_contracts_zone_session::v3::RoleBindingSpec;
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::bundle_resolver::BundleResolver;
    use d2b_core::execution_plan::{PrivateExecutionTable, PrivateExecutionValues};
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::{
        NodeId, ProcessNode, ProcessRole, ProcessesJson, RoleProfile, RoleUserNamespace,
        VmProcessDag, VmProcessInvariants,
    };
    use d2b_core::resource_authority::{AcceptedGraph, AcceptedSource};
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};

    use super::BundleExecutionValues;

    const ZONE: &str = "pubzone";
    const STORE: &str = "store-generation-1";
    const VOLUME: &str = "Volume/data";
    const CONSUMER: &str = "Process/shell";
    const SLOT: &str = "data";
    const PROVIDER: &str = "Provider/process";
    /// The trusted template the committed launch `Operation` names. It is the
    /// bundle's own per-role identity, and the private bundle below declares
    /// exactly one intent carrying it.
    const TEMPLATE: &str = "virtiofsd";
    const PROGRAM: &str = "/nix/store/2r1m-virtiofsd/bin/virtiofsd";
    const SOURCE_UID: &str = "11111111-1111-4111-8111-111111111111";
    const CONSUMER_UID: &str = "22222222-2222-4222-8222-222222222222";
    const STORE_ROOT: &str = "/var/lib/d2b/store";
    const RUNTIME_ROOT: &str = "/run/d2b";

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("the fixture references are canonical")
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("the fixture uids are canonical")
    }

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("the fixture tokens are canonical")
    }

    fn zone() -> d2b_contracts_resource::v3::ZoneId {
        d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("the fixture Zone is canonical")
    }

    fn binding_key() -> BindingKey {
        BindingKey::new(
            zone(),
            BindingKind::Volume,
            reference(VOLUME),
            uid(SOURCE_UID),
            reference(CONSUMER),
            uid(CONSUMER_UID),
            BindingSlot::parse(SLOT).expect("the fixture slot is a bounded token"),
        )
        .expect("the fixture relationship is well formed")
    }

    fn freshness(uid_value: &str) -> FreshnessTuple {
        FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse(STORE).expect("a bounded store generation"),
            reference(VOLUME),
            uid(uid_value),
            DesiredRevision::INITIAL
                .try_next()
                .expect("the desired revision has room"),
            DesiredDigest::of(uid_value.as_bytes()),
        )
    }

    // -----------------------------------------------------------------
    // The verified private bundle
    // -----------------------------------------------------------------

    fn host() -> HostJson {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": "v2",
            "site": { "allowUnsafeEastWest": false },
            "environments": [],
            "nftables": {
                "family": "inet",
                "table": "d2b",
                "chains": [],
                "tableHashAfterApply": null,
                "ownershipId": "test"
            },
            "networkManager": {
                "filePath": "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf",
                "matchCriteria": [],
                "reloadBehavior": "atomic-reload",
                "ownership": {
                    "owner": "root",
                    "group": "root",
                    "mode": "0644",
                    "driftPolicy": "replace"
                }
            },
            "hostsFile": {
                "startMarker": "# d2b-managed begin",
                "endMarker": "# d2b-managed end",
                "rule": "replace-managed-block"
            },
            "kernelModules": [],
            "fdOwnership": [],
            "cloudHypervisorCapabilities": [],
            "ifNameMappings": [],
            "qemuMedia": null,
            "ch": null,
            "firewallCoexistencePolicy": null
        }))
        .expect("the host fixture parses")
    }

    fn manifest() -> ManifestV04 {
        ManifestV04::from_slice(
            serde_json::to_vec(&serde_json::json!({
                "_manifest": { "manifestVersion": 6 },
                "_observability": {
                    "enabled": false,
                    "signozUrl": "http://127.0.0.1:8080",
                    "signozOtlpGrpcPort": 4317,
                    "signozOtlpHttpPort": 4318,
                    "obsVsockCid": 0,
                    "obsVsockHostSocket": "",
                    "vmName": ""
                }
            }))
            .expect("the manifest json serializes")
            .as_slice(),
        )
        .expect("the manifest fixture parses")
    }

    fn bundle() -> Bundle {
        Bundle {
            bundle_version: 4,
            schema_version: "v2".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        }
    }

    fn profile(role_id: &str) -> RoleProfile {
        RoleProfile {
            profile_id: format!("profile-{role_id}"),
            uid: 60_100,
            gid: 60_100,
            adr_carve_out: None,
            caps: Vec::new(),
            namespaces: NamespaceSet {
                mount: true,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: true,
            },
            seccomp_policy_ref: None,
            mount_policy: MountPolicy {
                read_only_paths: vec!["/nix/store".to_owned()],
                writable_paths: Vec::new(),
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
            },
            cgroup_placement: CgroupPlacement {
                subtree: format!("d2b.slice/vm-a/{role_id}"),
                controllers: vec!["cpu".to_owned()],
                delegated: false,
            },
            user_namespace: Some(RoleUserNamespace {
                host_uid_for_zero: 60_100,
                host_gid_for_zero: 60_100,
            }),
            umask: Some(0o007),
        }
    }

    fn runner_node(id: &str) -> ProcessNode {
        ProcessNode {
            id: NodeId(id.to_owned()),
            execution_ref: None,
            execution_domain: None,
            user_ref: None,
            role: ProcessRole::Virtiofsd,
            unit: None,
            binary_path: Some(PROGRAM.to_owned()),
            argv: vec![PROGRAM.to_owned(), "microvm-virtiofsd@vm-a".to_owned()],
            env: vec!["D2B_ROLE=virtiofsd".to_owned()],
            plan_ops: Vec::new(),
            network_interfaces: Vec::new(),
            profile: profile(id),
            readiness: Vec::new(),
        }
    }

    /// The private bundle's `processes.json`: one runner per VM.
    fn processes(vms: &[&str]) -> ProcessesJson {
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vms
                .iter()
                .map(|vm| VmProcessDag {
                    workload_identity: None,
                    vm: (*vm).to_owned(),
                    nodes: vec![runner_node(TEMPLATE)],
                    edges: Vec::new(),
                    invariants: VmProcessInvariants {
                        swtpm_pre_start_flush: false,
                        per_vm_audit_pipeline: false,
                        usbip_gating: true,
                        tpm_ownership_migration_without_running_vm_mutation: true,
                    },
                })
                .collect(),
        }
    }

    /// The Zone resource bundle: one declared `Volume` whose own source
    /// policy selects the `path:data` storage row.
    fn zone_bundle(volume_name: &str) -> Vec<u8> {
        let spec: CanonicalJsonObject =
            CanonicalJsonObject::parse(&serde_json::to_vec(&serde_json::json!({
                "source": {
                    "executionRef": "Host/host-system",
                    "settings": { "kind": "local-path", "sourcePolicyId": "data" }
                },
                "kind": "durable",
                "views": {
                    "root": { "path": "root", "rights": ["read"] }
                }
            }))
            .expect("the Volume spec serializes")
            .as_slice())
            .expect("the Volume spec is a canonical object");
        let resource = BundleResource::new(
            ResourceTypeName::parse("Volume").expect("`Volume` is a registered type"),
            BundleResourceMetadata::new(
                ResourceName::parse(volume_name)
                    .expect("the fixture name is canonical"),
                zone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            spec,
        )
        .expect("the bundle resource is well formed");
        let bundle = ResourceBundle::new(
            zone(),
            vec![resource],
            format!("sha256:{}", "b".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            d2b_contracts_resource::v3::Timestamp::parse("2026-01-01T00:00:00.000Z")
                .expect("the fixture timestamp is well formed"),
        )
        .expect("the Zone resource bundle is well formed");
        serde_json::to_vec(&bundle).expect("the Zone resource bundle serializes")
    }

    /// The trusted storage contract: the one row the declared `Volume`
    /// source policy selects.
    fn storage_contract() -> d2b_core::storage::StorageJson {
        use d2b_contracts::contract_id::{ContractId, PathTemplate};
        use d2b_core::storage::{
            ActorKind, ActorRef, CleanupPolicy, LeaseClass, PrincipalKind, PrincipalRef,
            RepairPolicy, SensitivityClass, StorageAdoptionPolicy, StorageInvariant, StorageJson,
            StorageLifecycle, StoragePathKind, StoragePathSpec, StoragePersistence,
            StorageRestartPolicy,
        };

        let principal =
            |kind: PrincipalKind, value: &str| PrincipalRef {
                kind,
                value: ContractId::parse(value).expect("the test principal id is bounded"),
            };
        let actor =
            |kind: ActorKind, value: &str| ActorRef {
                kind,
                value: ContractId::parse(value).expect("the test actor id is bounded"),
            };
        StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![StoragePathSpec {
                id: ContractId::parse("path:data").expect("the test storage id is bounded"),
                scope: ContractId::parse("vm:vm-a").expect("the test scope id is bounded"),
                path_template: PathTemplate::parse(STORE_ROOT)
                    .expect("the test path template is anchored"),
                kind: StoragePathKind::Directory,
                lifecycle: StorageLifecycle::BootScopedReadoptable,
                persistence: StoragePersistence::BootScoped,
                owner: principal(PrincipalKind::User, "d2bd"),
                group: principal(PrincipalKind::Group, "d2b"),
                mode: "0770".to_owned(),
                access_acl: Vec::new(),
                default_acl: Vec::new(),
                creator: actor(ActorKind::NixModule, "tmpfiles"),
                writers: vec![actor(ActorKind::Broker, "d2b-broker")],
                readers: Vec::new(),
                cleanup_policy: CleanupPolicy::Boot,
                repair_policy: RepairPolicy::NixActivation,
                restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
                adoption_policy: StorageAdoptionPolicy::AdoptWithLiveOwnerProof,
                lease_class: LeaseClass::None,
                sensitivity: SensitivityClass::Private,
                no_follow: true,
                recursive: false,
                invariants: vec![StorageInvariant::NoSymlink],
            }],
            restart_policies: Vec::new(),
            degraded_states: Vec::new(),
            remediations: Vec::new(),
        }
    }

    fn resolver(vms: &[&str], volume_name: &str) -> BundleResolver {
        let mut resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            bundle(),
            host(),
            processes(vms),
            manifest(),
            BTreeMap::from([("zones/pubzone/resource-bundle.json".to_owned(), zone_bundle(volume_name))]),
        );
        resolver.set_storage(storage_contract());
        resolver
    }

    // -----------------------------------------------------------------
    // The accepted graph and the committed contract
    // -----------------------------------------------------------------

    fn accepted_graph() -> AcceptedGraph {
        let zone = zone();
        AcceptedGraph::new(
            zone.clone(),
            StoreIncarnation::parse(STORE).expect("a bounded store generation"),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        )
        .with_role(
            reference("Role/reader"),
            AuthorizedRole::new(
                vec![RoleRule::new(
                    vec![ResourceTypeName::parse("VolumeBinding").expect("a registered type")],
                    vec![RoleResourceVerb::Create],
                    Vec::new(),
                    Vec::new(),
                    vec![zone],
                    Vec::new(),
                    Vec::new(),
                )
                .expect("the role rule validates")],
                Vec::new(),
            )
            .expect("the role validates"),
        )
        .with_role_binding(
            reference("RoleBinding/shell"),
            RoleBindingSpec::with_facets(
                reference("Role/reader"),
                vec![reference(CONSUMER)],
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            )
            .expect("the role binding validates"),
        )
        .with_source(AcceptedSource::new(
            SourceAdmission::new(
                binding_key(),
                vec![RequestedRights::Observe],
                BindingArbitration::Shared,
            )
            .expect("the source decision validates"),
            BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
                .expect("the realization support validates"),
        ))
    }

    /// The committed launch contract: the payload declares only a typed
    /// non-authority parameter, and the implementation is the bundle's own
    /// trusted executable template.
    fn operation() -> CallableOperation {
        let payload = PayloadSchema::parse(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "servingWorker": { "type": "boolean" } },
        }))
        .expect("the payload schema validates");
        let audit = OperationAudit::new(
            true,
            AuditMode::Yes,
            Vec::new(),
            Vec::new(),
            token("process-launch"),
        )
        .expect("the audit facet is bounded");
        let authority = OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            BoundedText::parse("d2b-launcher").expect("bounded text without control characters"),
            BrokerRequirement::Yes,
        );
        let fds = OperationFds::new(
            Vec::new(),
            vec![FdContract::new(
                token("pidfd"),
                FdKind::Pidfd,
                true,
            )],
            Vec::new(),
        )
        .expect("the fd contract is bounded");
        CallableOperation::new(
            OperationImplementation::trusted_executable_template(
                reference(PROVIDER),
                token(TEMPLATE),
            )
            .expect("a Provider reference is a declared implementation"),
            payload,
            None,
            true,
            SecretAccess::None,
            audit,
            None,
            authority,
            fds,
            OperationBounds::default(),
            PayloadProvenance::Request,
        )
        .expect("the committed contract is well formed")
    }

    /// The broker's private execution values for one Zone: the verified
    /// bundle, the broker's own runtime root, and the committed row state the
    /// broker observes.
    fn values(resolver: BundleResolver, committed: Vec<FreshnessTuple>) -> BundleExecutionValues {
        BundleExecutionValues::new(
            Arc::new(resolver),
            std::path::PathBuf::from(RUNTIME_ROOT),
            committed,
        )
    }

    fn resolved_table(resolver: BundleResolver) -> PrivateExecutionTable {
        let values = values(
            resolver,
            vec![freshness(SOURCE_UID), freshness(CONSUMER_UID)],
        );
        PrivateExecutionTable::from_authority(
            &accepted_graph(),
            vec![(reference("Operation/spawn-process"), operation())],
            &values,
        )
    }

    // -----------------------------------------------------------------
    // (a) POSITIVE: the table resolves a real effect's private values
    // -----------------------------------------------------------------

    /// The resolver joins the accepted graph with the verified bundle and
    /// produces a table the plan resolver can plan against: every private
    /// value is the bundle's, and every one of them is read out of the table
    /// rather than out of the request.
    #[test]
    fn the_table_resolves_an_effect_against_the_verified_bundle() {
        let table = resolved_table(resolver(&["vm-a"], "data"));

        let source = table
            .source(&uid(SOURCE_UID))
            .expect("the declared Volume resolves a private source");
        assert_eq!(source.reference(), &reference(VOLUME));
        assert_eq!(
            source.backing_path().as_path().to_string_lossy(),
            format!("{STORE_ROOT}/data"),
            "the backing path is the storage contract's own row",
        );
        assert_eq!(
            source
                .views()
                .iter()
                .map(|view| (
                    view.name().as_str().to_owned(),
                    view.path().as_path().to_string_lossy().into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![("root".to_owned(), format!("{STORE_ROOT}/data/root"))],
            "the view is the declared view under the same storage row",
        );

        let destinations = table.destinations(&binding_key().address());
        assert_eq!(destinations.len(), 1);
        assert_eq!(
            destinations[0].path().as_path().to_string_lossy(),
            format!("{RUNTIME_ROOT}/effects/{ZONE}/shell/{SLOT}"),
            "the destination is the broker's own tree, keyed by the relationship",
        );

        let executable = table
            .executable(&reference("Operation/spawn-process"))
            .expect("the declared Operation resolves a trusted executable");
        assert_eq!(
            executable.program().as_path().to_string_lossy(),
            PROGRAM,
            "the program is the verified bundle's own binary",
        );
        assert_eq!(
            executable.argv(),
            [PROGRAM.to_owned(), "microvm-virtiofsd@vm-a".to_owned()],
            "argv[0] is the bundle's program and the rest is its own vector",
        );
        assert_eq!(
            executable.environment(),
            ["D2B_VM=vm-a", "D2B_ROLE=virtiofsd"],
            "the environment is the bundle's own baseline plus its node entries",
        );
        assert_eq!(
            executable.implementation(),
            operation().implementation(),
            "the resolved executable is exactly the committed implementation",
        );

        assert!(
            table.observed(&uid(SOURCE_UID)).is_some(),
            "the source's committed state is fenced",
        );
        assert!(
            table.observed(&uid(CONSUMER_UID)).is_some(),
            "the consumer's committed state is fenced",
        );
    }

    /// The whole chain, end to end: the broker's resolver produces a table,
    /// the plan resolver plans against it, and the answer carries the
    /// bundle's own program, arguments, and environment rather than anything
    /// a request named.
    #[test]
    fn an_admitted_effect_resolves_its_plan_against_the_bundle() {
        use d2b_core::execution_plan::{
            BindingPlanRequest, EffectPlanRequest, admit_parameters, resolve_execution_plan,
        };
        use d2b_core::resource_authority::TransportIdentity;

        let graph = accepted_graph();
        let table = resolved_table(resolver(&["vm-a"], "data"));
        let callable = operation();
        let parameters = admit_parameters(
            &callable,
            &CanonicalJsonObject::parse(
                &serde_json::to_vec(&serde_json::json!({ "servingWorker": true }))
                    .expect("the parameters serialize"),
            )
            .expect("the parameters are a canonical object"),
        )
        .expect("the typed parameters are admitted");

        let request = EffectPlanRequest::new(
            reference("Operation/spawn-process"),
            callable,
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            vec![BindingPlanRequest::new(
                binding_key(),
                RequestedRights::Observe,
                vec![BindingRealizationFacet::FilesystemPresentation],
                None,
            )],
            parameters,
            vec![freshness(SOURCE_UID), freshness(CONSUMER_UID)],
            TransportIdentity::Broker,
            None,
        )
        .expect("the plan request is well formed");

        let plan = resolve_execution_plan(&request, &graph, &table)
            .expect("the broker resolves the plan from its own authority");
        assert_eq!(
            plan.executable().program().as_path().to_string_lossy(),
            PROGRAM,
            "the plan carries the bundle's program, not one a caller named",
        );
        assert_eq!(
            plan.executable().environment(),
            ["D2B_VM=vm-a", "D2B_ROLE=virtiofsd"],
        );
        assert_eq!(plan.destinations().len(), 1);
        assert_eq!(plan.sources().len(), 1);
        assert_eq!(
            plan.freshness().observed().len(),
            2,
            "the plan is fenced on both committed rows",
        );
    }

    // -----------------------------------------------------------------
    // (b) NEGATIVE: the table refuses what it has no authority for
    // -----------------------------------------------------------------

    /// A template the verified bundle declares for more than one execution
    /// resolves to nothing.
    ///
    /// This is the refusal that matters most. The admitted carrier names the
    /// `Operation` and the relationship, not which execution it runs for, so
    /// two VMs each declaring the same template is exactly the case where
    /// "resolve the first one the bundle holds" would run the wrong process.
    #[test]
    fn an_ambiguous_template_resolves_no_executable() {
        let table = resolved_table(resolver(&["vm-a", "vm-b"], "data"));
        assert!(
            table
                .executable(&reference("Operation/spawn-process"))
                .is_none(),
            "a template two VMs declare resolves no executable",
        );
    }

    /// A template the verified bundle does not declare resolves to nothing.
    ///
    /// The committed contract names a trusted template; the bundle carries no
    /// intent for it; the answer is no program rather than an inherited one.
    #[test]
    fn a_template_the_bundle_does_not_declare_resolves_no_executable() {
        let resolver = resolver(&["vm-a"], "data");
        // Rename every intent the bundle carries to a template no committed
        // contract names, so the bundle is intact and simply has nothing for
        // this operation.
        let intent_id = resolver
            .runner_intent_ids()
            .next()
            .expect("the fixture bundle declares one runner intent")
            .to_owned();
        let table = PrivateExecutionTable::from_authority(
            &accepted_graph(),
            vec![(
                reference("Operation/spawn-process"),
                declared_for("swtpm"),
            )],
            &values(resolver, vec![freshness(SOURCE_UID), freshness(CONSUMER_UID)]),
        );
        assert!(
            table
                .executable(&reference("Operation/spawn-process"))
                .is_none(),
            "the bundle declares no intent for `swtpm`, so nothing is resolved ({intent_id})",
        );
    }

    /// A `Volume` the verified Zone bundle does not declare resolves no
    /// private source.
    #[test]
    fn a_volume_the_zone_bundle_does_not_declare_resolves_no_source() {
        let table = resolved_table(resolver(&["vm-a"], "other"));
        assert!(
            table.source(&uid(SOURCE_UID)).is_none(),
            "a Volume the verified Zone bundle does not declare has no private path",
        );
    }

    /// The refusal survives the removal of the refusal.
    ///
    /// This is the negative proof's own check. `BundleExecutionValues` is
    /// asked for the executable of a contract the bundle declares twice, and
    /// the answer must be `None`. If the refusal were deleted - if the lookup
    /// fell back to the first intent the bundle holds - this case fails,
    /// because the answer would become `Some` and the effect would run
    /// `vm-a`'s runner for a request that named neither VM.
    #[test]
    fn removing_the_ambiguity_refusal_would_serve_the_wrong_execution() {
        let resolver = Arc::new(resolver(&["vm-a", "vm-b"], "data"));
        let values = BundleExecutionValues::new(
            Arc::clone(&resolver),
            std::path::PathBuf::from(RUNTIME_ROOT),
            Vec::new(),
        );

        // What the refusal path returns.
        assert!(
            values
                .executable(&reference("Operation/spawn-process"), &operation())
                .is_none(),
            "two VMs declare the template, so no executable resolves",
        );

        // What a deleted refusal would return: the first intent the bundle
        // happens to hold. This is the whole of what the refusal prevents.
        let first = resolver
            .runner_intent_ids()
            .next()
            .expect("the fixture bundle declares runner intents");
        let passthrough = resolver
            .find_runner_intent(first)
            .expect("the bundle carries the intent a deleted refusal would pick");
        assert_eq!(
            passthrough.binary_path.to_string_lossy(),
            PROGRAM,
            "a deleted refusal would execute whichever VM's intent sorted first",
        );
    }

    /// The committed `Operation` contract for one trusted template.
    fn declared_for(template: &str) -> CallableOperation {
        let payload = PayloadSchema::parse(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "servingWorker": { "type": "boolean" } },
        }))
        .expect("the payload schema validates");
        let audit = OperationAudit::new(
            true,
            AuditMode::Yes,
            Vec::new(),
            Vec::new(),
            token("process-launch"),
        )
        .expect("the audit facet is bounded");
        let authority = OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            BoundedText::parse("d2b-launcher").expect("bounded text without control characters"),
            BrokerRequirement::Yes,
        );
        let fds = OperationFds::new(Vec::new(), Vec::new(), Vec::new())
            .expect("the fd contract is bounded");
        CallableOperation::new(
            OperationImplementation::trusted_executable_template(reference(PROVIDER), token(template))
                .expect("a Provider reference is a declared implementation"),
            payload,
            None,
            true,
            SecretAccess::None,
            audit,
            None,
            authority,
            fds,
            OperationBounds::default(),
            PayloadProvenance::Request,
        )
        .expect("the committed contract is well formed")
    }

    /// The committed row state the broker observes is an input, and an absent
    /// one refuses the effect rather than defaulting a revision.
    #[test]
    fn an_unobserved_committed_row_resolves_no_source() {
        let table = PrivateExecutionTable::from_authority(
            &accepted_graph(),
            vec![(reference("Operation/spawn-process"), operation())],
            // No committed row state at all: the AE16 fence has nothing to
            // compare against.
            &values(resolver(&["vm-a"], "data"), Vec::new()),
        );
        assert!(
            table.source(&uid(SOURCE_UID)).is_none(),
            "a relationship whose committed state the broker cannot observe has no entry",
        );
    }

    /// A row state from another store generation is a different store, not an
    /// older but current one, so it fences nothing.
    #[test]
    fn a_row_state_from_another_store_generation_resolves_no_source() {
        let other_store = FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-generation-0").expect("a bounded store generation"),
            reference(VOLUME),
            uid(SOURCE_UID),
            DesiredRevision::INITIAL,
            DesiredDigest::of(SOURCE_UID.as_bytes()),
        );
        let table = PrivateExecutionTable::from_authority(
            &accepted_graph(),
            vec![(reference("Operation/spawn-process"), operation())],
            &values(
                resolver(&["vm-a"], "data"),
                vec![other_store, freshness(CONSUMER_UID)],
            ),
        );
        assert!(
            table.source(&uid(SOURCE_UID)).is_none(),
            "a tuple from another store generation is not an observation of this one",
        );
    }

    /// The right an effect runs under is never one the request named, and the
    /// closed set of subjects the table carries identities for is the graph's
    /// own.
    #[test]
    fn no_identity_is_resolved_for_a_subject_the_bundle_does_not_name() {
        let values = values(resolver(&["vm-a"], "data"), Vec::new());
        assert!(
            values.identity(&reference(CONSUMER)).is_none(),
            "the verified bundle keys runner identities by execution target, not by a bare \
             subject reference, so it resolves none",
        );
    }
}
