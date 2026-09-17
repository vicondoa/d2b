//! The Network resource driver: the v3 `ResourceDriver` conversion of the
//! daemon-owned network-local Provider path.
//!
//! The family serves one ResourceType (`Network`) with one declared row
//! (`Provider/network-local`) and one typed effect behind
//! [`NetworkDriverEffects`]: the daemon realizes the row through the
//! preserved Network reconciler and the typed broker adapter, while this
//! crate owns the driver, its declaration, the child set it derives (the
//! config Volume, the net-VM Guest, and the in-guest network agent Process),
//! and the dependency references it watches.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`ProviderRow`] registration under the family's
//!   ResourceTypes.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the spec decodes and
//!   names a Provider this family owns for the row's ResourceType.
//! - `observe` -> [`ResourceDriver::recover`]: owned-child adoption.
//! - finalizer enrollment + `plan`/`reconcile`/`execute_effect` ->
//!   [`ResourceDriver::reconcile`]: the desired child set is ensured through
//!   the manager child API (committed before the child actor exists, F1), the
//!   typed Provider effect runs behind the port, and the in-memory status
//!   projection is published with `ctx.set_status` (R11) plus a self-requeue
//!   while the family is not converged.
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the family's staged fabric finalizer runs
//!   before the owned children retire.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceRef, ResourceUid, execution_policy::ExecutionPolicy,
    guest::GuestSpec, network::NetworkSpec,
};
use d2b_provider_toolkit::{
    ProviderRow, SharedProviderDeclarationError, SharedProviderDriverArgs,
    SharedProviderDriverFactory, SharedProviderEffectError, SharedProviderEffectOutcome,
    SharedProviderEffectRequest, SharedProviderFamily, SharedProviderFinalize, VolumeAnchorRefresh,
    key_ref, resource_uid, shared_provider_spec_decoder,
};
use d2b_resource_runtime::context::{ChildEnsure, ResourceContext};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_types::{
    AllowedSources, ChildCreation, ChildCustody, DriverDescriptor, WellKnownType,
};
use serde_json::{Value, json};

use crate::operations::network_family_operations;

/// The Network ResourceType served by the network-local Provider.
pub const NETWORK_TYPE_NAME: &str = "Network";

/// The Provider identity this family's row declares.
pub const NETWORK_PROVIDER_REF: &str = "Provider/network-local";

/// The controller reference the row's effects bind.
pub const NETWORK_CONTROLLER_REF: &str = "Process/network-local-controller";

/// Preserved reconcile self-resync for rows whose Provider is not converged
/// (old shared Runner repair interval for the Network Provider).
pub const NETWORK_RESYNC: Duration = Duration::from_secs(30);

/// The config Volume the Network family declares for a net VM.
const CONFIG_VOLUME_PROVIDER_REF: &str = "Provider/volume-local";

/// The network agent Process Provider the family declares.
const AGENT_PROCESS_PROVIDER_REF: &str = d2b_provider_process_minijail::PROVIDER_REF;

/// The net-VM Guest Provider the family declares.
const NET_VM_GUEST_PROVIDER_REF: &str = d2b_provider_guest_cloud_hypervisor::PROVIDER_REF;

/// The Network family's closed component vocabulary.
///
/// The family serves exactly one registration row today; the vocabulary is
/// the declaration the row's handler is selected by, so a second Network
/// registration would name its own component instead of arriving as a
/// type-name heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkComponent {
    /// The `Network` row served by `Provider/network-local`.
    Network,
}

/// The rows this family declares, in the preserved registration order.
pub const NETWORK_REGISTRATIONS: [ProviderRow<NetworkComponent>; 1] = [ProviderRow {
    resource_type: NETWORK_TYPE_NAME,
    component: NetworkComponent::Network,
    controller_ref: NETWORK_CONTROLLER_REF,
    provider_ref: NETWORK_PROVIDER_REF,
    effect_id: "network",
    resync: NETWORK_RESYNC,
}];

/// The children the Network family creates, as declarations.
///
/// The config Volume, the net-VM Guest, and the in-guest network agent
/// Process are the derived refs the old shared Runner materialized
/// (`SharedRunnerNetworkResources`, unchanged).
pub const NETWORK_CREATIONS: [ChildCreation; 3] = [
    ChildCreation {
        child: WellKnownType::VOLUME,
        provider_ref: CONFIG_VOLUME_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 0,
    },
    ChildCreation {
        child: WellKnownType::GUEST,
        provider_ref: NET_VM_GUEST_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 1,
    },
    ChildCreation {
        child: WellKnownType::PROCESS,
        provider_ref: AGENT_PROCESS_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 2,
    },
];

/// The Provider effect surface the Network driver needs.
///
/// The production implementation owns the preserved Network reconciler, the
/// broker-backed fabric effects, and the typed Network effect port; test
/// doubles implement the same seam.
#[async_trait]
pub trait NetworkDriverEffects: Send + Sync + 'static {
    /// Reconcile one Network row through the Network-local controller.
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one Network row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;

    /// Re-register the plane's per-resource Volume anchors after the driver
    /// committed a Volume child row.
    async fn refresh_volume_anchors(&self) {}
}

/// Everything the composition must construct to instantiate the Network
/// driver factory for one zone.
pub struct NetworkDriverArgs {
    /// The zone the driver serves.
    pub zone: String,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-realized effect port the driver drives.
    pub effects: Arc<dyn NetworkDriverEffects>,
}

/// The anchor-refresh adapter handed to the child surface.
///
/// The child surface is toolkit-owned and cannot name this family's port, so
/// the family bridges its `refresh_volume_anchors` hook into the neutral
/// handle.
struct NetworkAnchorRefresh(Arc<dyn NetworkDriverEffects>);

#[async_trait]
impl VolumeAnchorRefresh for NetworkAnchorRefresh {
    async fn refresh_volume_anchors(&self) {
        self.0.refresh_volume_anchors().await;
    }
}

/// The family's declarations and typed Provider effect.
struct NetworkFamily {
    effects: Arc<dyn NetworkDriverEffects>,
}

#[async_trait]
impl SharedProviderFamily for NetworkFamily {
    type Component = NetworkComponent;
    type State = ();

    fn rows(&self) -> &'static [ProviderRow<Self::Component>] {
        &NETWORK_REGISTRATIONS
    }

    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: NetworkComponent,
        spec: &Value,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
        match component {
            NetworkComponent::Network => {
                let owner = key_ref(ctx.key());
                let uid =
                    resource_uid(ctx.uid()).map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                let spec =
                    network_spec(spec).map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                network_child_ensures(&owner, &uid, &spec)
            }
        }
    }

    fn declared_dependency_refs(
        &self,
        component: NetworkComponent,
        spec: &Value,
        metadata: &Value,
    ) -> Vec<ResourceRef> {
        declared_dependency_refs(component, spec, metadata)
    }

    async fn effect(
        &self,
        _component: NetworkComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.effects.reconcile_network(request).await
    }

    async fn finalize(
        &self,
        _component: NetworkComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.effects.finalize(request).await
    }

    fn volume_anchor_refresh(&self) -> Option<Arc<dyn VolumeAnchorRefresh>> {
        Some(Arc::new(NetworkAnchorRefresh(Arc::clone(&self.effects))))
    }
}

/// The resource verbs the Network type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs, which the plane gates to the `Credential` type.
const NETWORK_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
];

/// The execution domains the Network type can be reconciled in.
const NETWORK_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Network realization reads while reconciling: the
/// Host fabric, the net-VM Guest, the config Volume, the in-guest agent
/// Process, and the attachment Users the reconciler gates on.
const NETWORK_READS: &[WellKnownType] = &[
    WellKnownType::HOST,
    WellKnownType::GUEST,
    WellKnownType::VOLUME,
    WellKnownType::PROCESS,
    WellKnownType::USER,
];

/// The Network type's driver declaration.
///
/// `Network` is `BUILTIN | STARTUP` (no RUNTIME bit): zone networking is
/// referenced by every Guest-bearing zone, so the plane must have the driver
/// registered before it opens. The type is not exportable: `ResourceExport`
/// admits only qualified `*.d2bus.org.*Service` types, so a network can never
/// be an export subject. The driver declares the thirteen network-fds family
/// operations (U12): the broker-generic kernels serve each operation's
/// privileged core in-broker, while the family operation itself stays
/// forwarded to this declaring process. The children it derives are declared
/// in [`NETWORK_CREATIONS`].
pub fn network_descriptor(args: NetworkDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::NETWORK,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: NETWORK_VERBS,
        execution: NETWORK_EXECUTION_DOMAINS,
        exportable: false,
        reads: NETWORK_READS,
        operations: network_family_operations(),
        creations: &NETWORK_CREATIONS,
        startup: &[],
        services: &[],
        decoder: shared_provider_spec_decoder(),
        factory: Arc::new(SharedProviderDriverFactory::new(
            SharedProviderDriverArgs {
                zone: args.zone,
                controller_generation: args.controller_generation,
                family: Arc::new(NetworkFamily {
                    effects: args.effects,
                }),
            },
        )),
    }
}

/// The typed Network contract of one stored spec.
fn network_spec(spec: &Value) -> Result<NetworkSpec, ()> {
    let mut spec_value = spec.clone();
    if let Some(spec) = spec_value.as_object_mut() {
        for field in ["providerRef", "updatePolicy", "provider"] {
            spec.remove(field);
        }
    }
    serde_json::from_value(spec_value).map_err(|_| ())
}

/// The Network family's desired children: the config Volume, the net-VM
/// Guest, and the in-guest network agent Process (old
/// `SharedRunnerNetworkResources` derived refs, unchanged).
fn network_child_ensures(
    owner: &ResourceRef,
    network_uid: &ResourceUid,
    spec: &NetworkSpec,
) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
    let error = || SharedProviderDeclarationError::SpecInvalid;
    let vm_name = crate::ifname::derive_network_child_name(network_uid, "vm");
    let agent_name = crate::ifname::derive_network_child_name(network_uid, "agent");
    let metadata = serde_json::to_vec(&json!({
        "ownerRef": owner.to_canonical_string(),
        "labels": {},
        "annotations": {},
    }))
    .map_err(|_| error())?;

    let volume_spec = crate::controller::config_volume_spec("host-system", Some(&vm_name))
        .map_err(|_| error())?;
    let mut volume = serde_json::to_value(&volume_spec).map_err(|_| error())?;
    volume
        .as_object_mut()
        .ok_or_else(error)?
        .insert(
            "providerRef".to_owned(),
            Value::String(CONFIG_VOLUME_PROVIDER_REF.to_owned()),
        );

    let artifact = crate::artifact::resolve_net_vm_system_artifact(
        spec,
        &[crate::artifact::ArtifactCatalogEntry::new(
            spec.net_vm_system_artifact_id().clone(),
            crate::artifact::ArtifactKind::NixosSystem,
        )],
    )
    .map_err(|_| error())?;
    let guest = GuestSpec::new(ExecutionPolicy::system_default(), Some(artifact));
    let mut guest_value = serde_json::to_value(&guest).map_err(|_| error())?;
    guest_value
        .as_object_mut()
        .ok_or_else(error)?
        .insert(
            "providerRef".to_owned(),
            Value::String(NET_VM_GUEST_PROVIDER_REF.to_owned()),
        );

    let agent = crate::controller::guest_agent_process_spec(&vm_name).map_err(|_| error())?;
    let mut agent_value = serde_json::to_value(&agent).map_err(|_| error())?;
    agent_value
        .as_object_mut()
        .ok_or_else(error)?
        .insert(
            "providerRef".to_owned(),
            Value::String(AGENT_PROCESS_PROVIDER_REF.to_owned()),
        );

    Ok(Some(vec![
        ChildEnsure {
            type_name: ResourceTypeName::new("Volume"),
            name: "net-config".to_owned(),
            spec: serde_json::to_vec(&volume).map_err(|_| error())?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Guest"),
            name: vm_name.clone(),
            spec: serde_json::to_vec(&guest_value).map_err(|_| error())?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Process"),
            name: agent_name,
            spec: serde_json::to_vec(&agent_value).map_err(|_| error())?,
            metadata,
        },
    ]))
}

/// The dependency references one Network row declares: every attachment's
/// execution target (old dependency selectors, unchanged).
pub fn declared_dependency_refs(
    _component: NetworkComponent,
    spec: &Value,
    _metadata: &Value,
) -> Vec<ResourceRef> {
    let mut refs = Vec::new();
    if let Some(attachments) = spec.pointer("/spec/attachments").and_then(Value::as_array) {
        for attachment in attachments {
            if let Some(reference) = attachment
                .get("executionRef")
                .and_then(Value::as_str)
                .and_then(|value| ResourceRef::parse(value).ok())
            {
                refs.push(reference);
            }
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::network::{Ipv4Cidr, NetworkSpec};
    use d2b_provider_toolkit::SharedProviderSpecEnvelope;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::error::ResourceError;
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use serde_json::json;

    use super::{
        NETWORK_PROVIDER_REF, NETWORK_TYPE_NAME, NetworkDriverArgs, declared_dependency_refs,
        network_descriptor, network_spec,
    };
    use crate::test_support::RecordingEffects;

    /// Ordered log the fixture writes, so ordering is one assertion.
    type Log = Arc<tokio::sync::Mutex<Vec<String>>>;

    struct RecordingManager {
        log: Log,
        owned: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
    }

    impl RecordingManager {
        fn new(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                owned: tokio::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.log
                .lock()
                .await
                .push(format!("ensure:{}/{}", child.type_name.as_str(), child.name));
            Ok(EnsureOutcome::Created(test_row(
                child.type_name.as_str(),
                &child.name,
            )))
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .owned
                .lock()
                .await
                .iter()
                .find(|row| row.key == *key)
                .cloned())
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock()
                .await
                .push(format!("delete:{}/{}", key.type_name, key.name));
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.owned.lock().await.clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingRequeue {
        scheduled: parking_lot::Mutex<Vec<RequeueId>>,
    }

impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, _after: Duration) -> RequeueId {
            let mut scheduled = self.scheduled.lock();
            let id = RequeueId(scheduled.len() as u64 + 1);
            scheduled.push(id);
            id
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    fn test_row(type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("dev", type_name, name),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"spec".to_vec(),
            metadata: Vec::new(),
            created_at: 1_725_000_000,
        }
    }

    fn network_spec_value(provider_ref: &str) -> serde_json::Value {
        let spec = NetworkSpec::minimal(
            Ipv4Cidr::parse("10.20.0.0/24").expect("lan"),
            Ipv4Cidr::parse("192.0.2.0/30").expect("uplink"),
            BoundedToken::parse("net-vm-base").expect("token"),
        )
        .expect("network spec");
        let mut value = serde_json::to_value(&spec).expect("network spec json");
        value
            .as_object_mut()
            .expect("spec object")
            .insert("providerRef".to_owned(), json!(provider_ref));
        value
    }

    fn descriptor(effects: Arc<RecordingEffects>) -> d2b_resource_types::DriverDescriptor {
        network_descriptor(NetworkDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
                .expect("generation"),
            effects,
        })
    }

    #[allow(clippy::type_complexity)]
    fn context(
        descriptor: &d2b_resource_types::DriverDescriptor,
        spec: serde_json::Value,
        manager: Arc<RecordingManager>,
    ) -> d2b_resource_runtime::context::ResourceContext {
        let mut row = test_row(NETWORK_TYPE_NAME, "net-main");
        row.spec = serde_json::to_vec(&spec).expect("spec");
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        d2b_resource_runtime::context::ResourceContext::new(
            row,
            TargetHandle::Host,
            descriptor.decoder.clone(),
            manager,
            Arc::new(RecordingRequeue::default()),
            effects_tx,
            notify_tx,
        )
    }

    /// The declaration serves exactly the Network type over the family's own
    /// effect port, and it declares the three children the family derives.
    #[test]
    fn descriptor_declares_the_network_type_and_its_children() {
        let descriptor = descriptor(Arc::new(RecordingEffects::default()));
        assert_eq!(
            descriptor.resource_type.to_resource_type_name().as_str(),
            NETWORK_TYPE_NAME
        );
        let types = descriptor
            .factory
            .resource_types()
            .iter()
            .map(|resource_type| resource_type.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(types, vec![NETWORK_TYPE_NAME.to_owned()]);
        let creations = descriptor
            .creations
            .iter()
            .map(|creation| creation.child.to_resource_type_name().as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            creations,
            vec!["Volume".to_owned(), "Guest".to_owned(), "Process".to_owned()]
        );
    }

    /// A reconcile pass commits the declared child set before the typed
    /// effect runs, and the whole set is derived from the row's own identity.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_commits_the_declared_children_before_the_effect() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let effects = Arc::new(RecordingEffects::default());
        let descriptor = descriptor(Arc::clone(&effects));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(
            &descriptor,
            network_spec_value(NETWORK_PROVIDER_REF),
            manager,
        );
        let mut driver = descriptor.factory.create(ctx.key()).await;
        driver.validate(&mut ctx).await.expect("network row validates");
        driver
            .reconcile(&mut ctx)
            .await
            .expect("network row reconciles");

        let uid = d2b_provider_toolkit::resource_uid(&[0x42; 16]).expect("uid");
        let vm = crate::ifname::derive_network_child_name(&uid, "vm");
        let agent = crate::ifname::derive_network_child_name(&uid, "agent");
        let entries = log.lock().await.clone();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("ensure:"))
                .count(),
            3,
            "the declared child set is committed: {entries:?}"
        );
        for expected in [
            "ensure:Volume/net-config".to_owned(),
            format!("ensure:Guest/{vm}"),
            format!("ensure:Process/{agent}"),
        ] {
            assert!(entries.contains(&expected), "{entries:?}");
        }
        assert_eq!(*effects.reconciled.lock(), 1);
    }

    /// The family decoder yields the shared envelope the driver's verbs read.
    #[test]
    fn the_decoder_yields_the_shared_envelope() {
        let descriptor = descriptor(Arc::new(RecordingEffects::default()));
        let bytes = serde_json::to_vec(&network_spec_value(NETWORK_PROVIDER_REF)).expect("spec");
        let decoded = descriptor.decoder.decode(&bytes).expect("decode");
        let envelope = decoded
            .downcast_ref::<SharedProviderSpecEnvelope>()
            .expect("the family decoder yields the shared envelope");
        assert_eq!(envelope.provider_ref(), Some(NETWORK_PROVIDER_REF));
    }

    /// The typed Network contract strips the envelope-only fields exactly as
    /// the old daemon driver did.
    #[test]
    fn network_spec_strips_the_envelope_fields() {
        let spec = network_spec(&network_spec_value(NETWORK_PROVIDER_REF)).expect("typed spec");
        let reencoded = serde_json::to_value(&spec).expect("spec json");
        assert!(reencoded.get("providerRef").is_none());
        assert!(network_spec(&json!({"providerRef": NETWORK_PROVIDER_REF})).is_err());
    }

    /// The family watches every attachment's execution target.
    #[test]
    fn dependency_refs_are_the_attachment_execution_targets() {
        let spec = json!({
            "spec": {
                "attachments": [
                    {"executionRef": "Guest/dev-vm-0"},
                    {"executionRef": "Guest/dev-vm-1"}
                ]
            }
        });
        let refs = declared_dependency_refs(super::NetworkComponent::Network, &spec, &json!({}));
        assert_eq!(
            refs.iter()
                .map(|reference| reference.to_canonical_string())
                .collect::<Vec<_>>(),
            vec!["Guest/dev-vm-0".to_owned(), "Guest/dev-vm-1".to_owned()]
        );
    }

    /// A row naming a Provider outside the family is terminal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_rejects_a_foreign_provider() {
        let descriptor = descriptor(Arc::new(RecordingEffects::default()));
        let manager = RecordingManager::new(Arc::new(tokio::sync::Mutex::new(Vec::new())));
        let mut ctx = context(
            &descriptor,
            network_spec_value("Provider/network-other"),
            manager,
        );
        let mut driver = descriptor.factory.create(ctx.key()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("must refuse");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::terminal(
                d2b_resource_runtime::error::DriverOp::Validate
            )
        );
    }

    /// The declaration's row carries the preserved controller/provider
    /// identity and the preserved resync cadence.
    #[test]
    fn the_row_keeps_the_preserved_identity() {
        let row = &super::NETWORK_REGISTRATIONS[0];
        assert_eq!(row.resource_type, NETWORK_TYPE_NAME);
        assert_eq!(row.provider_ref, NETWORK_PROVIDER_REF);
        assert_eq!(row.effect_id, "network");
        assert_eq!(row.resync, super::NETWORK_RESYNC);
        assert_eq!(
            ResourceTypeName::new(row.resource_type).as_str(),
            NETWORK_TYPE_NAME
        );
    }

    /// The shared recording double appends every effect call in order, so the
    /// plane can assert reconcile-then-finalize ordering through `call_order()`
    /// while the per-verb counters keep counting.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recording_effects_records_ordered_calls() {
        let effects = Arc::new(RecordingEffects::default());
        let descriptor = descriptor(Arc::clone(&effects));
        let manager = RecordingManager::new(Arc::new(tokio::sync::Mutex::new(Vec::new())));
        let mut ctx = context(
            &descriptor,
            network_spec_value(NETWORK_PROVIDER_REF),
            manager,
        );
        let mut driver = descriptor.factory.create(ctx.key()).await;
        driver.validate(&mut ctx).await.expect("network row validates");
        driver.reconcile(&mut ctx).await.expect("network row reconciles");
        driver.delete(&mut ctx).await.expect("network row finalizes");
        assert_eq!(effects.call_order(), vec!["reconcile", "finalize"]);
        assert_eq!(*effects.reconciled.lock(), 1);
        assert_eq!(*effects.finalized.lock(), 1);
    }
}

