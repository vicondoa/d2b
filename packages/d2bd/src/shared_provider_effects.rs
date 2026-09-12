//! Production effects for the U12 shared host-provider drivers.
//!
//! Every typed Provider effect the driver family dispatches lives here: the
//! Network-local controller over a manager-routed child port, the persistent
//! TPM controller, the USBIP lifecycle over the broker authority ledger, the
//! SecurityKey relay lifecycle, and the authority-fenced GPU lifecycle. The
//! port is the dyn-erased [`SharedProviderDriverEffects`] boundary; the
//! daemon owns every side effect behind it (broker dispatch, device grants,
//! the pidfd table, authority leases) and the drivers own the child rows.
//!
//! Live readiness is read through the manager view for converted rows (a
//! converted row's actor status is the only status there is, R11) and through
//! the durable store for unconverted rows - the same split the old effects
//! got from `/status/phase`. The driver never sees either.

use std::collections::BTreeSet;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts::types::{BundleOpId, VmId};
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    identity::ReconnectGeneration, network::NetworkProvenance, volume::VolumeSpec,
};
use d2b_core_controller::authority::AuthorityRequest;
use d2b_provider_network_local::{
    artifact::{ArtifactCatalogEntry, ArtifactKind},
    controller::{
        AttachmentRealization, NetworkAdmissionIntent, NetworkAdmissionKey, NetworkAdmissionProof,
        NetworkEffectError, NetworkReconciler, NetworkResourcePort, ReconcileInput,
        ReconcileProgress,
    },
    observe::observe_host_network,
};
use d2b_resource_runtime::context::ChildEnsure;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::ResourceStatus;
use d2b_resource_store::{
    ResourceAssignmentFence, ResourceAssignmentScope, StoreErrorKind, StoreGetRequest,
    StoreOperationContext, StoreProjection,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ServerState;
use crate::resource_plane_v3::{PlaneRoute, ResourcePlaneV3, route_resource_type};
use crate::resource_runtime::{ASSIGNMENT_EPOCH, ZoneResourceRuntime};
use crate::shared_provider_driver::{
    SharedProviderResourceState,
    HOST_REF, SecurityKeyComponent, SharedProviderDriverEffects, SharedProviderEffectError,
    SharedProviderEffectOutcome, SharedProviderEffectPhase, SharedProviderEffectRequest,
    SharedProviderFinalize, SharedProviderKind, UsbipComponent,
};
use crate::usbip_production::UsbipChildResourcePort;

/// Production composition adapter for the closed shared-provider family.
///
/// The adapter performs the Provider-owned typed admission before any effect
/// call. A missing live broker/resource binding is returned as a retryable
/// refusal; it is never converted into generic convergence.
pub(crate) struct ProductionSharedProviderEffects {
    state: Arc<ServerState>,
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    /// Zone-wide USBIP authority ledger (old `usbip_ledger`), shared by every
    /// USBIP Service and Binding dispatcher in the zone.
    usbip_ledger: Arc<Mutex<crate::usbip_production::AuthorityLedger>>,
    /// Zone-wide activated USBIP services (old `usbip_services`).
    usbip_services: Arc<Mutex<BTreeSet<ResourceUid>>>,
}

impl ProductionSharedProviderEffects {
    pub(crate) fn new(
        state: Arc<ServerState>,
        zone: ZoneId,
        controller_generation: ControllerGeneration,
    ) -> Self {
        Self {
            state,
            zone,
            controller_generation,
            usbip_ledger: crate::usbip_production::new_authority_ledger(),
            usbip_services: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, SharedProviderEffectError> {
        self.state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(SharedProviderEffectError::Unavailable)
    }

    /// The published v3 plane (manager-backed live rows and status).
    fn plane(&self) -> Result<Arc<ResourcePlaneV3>, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        runtime
            .v3_plane()
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    fn operation(&self, operation_id: &str) -> StoreOperationContext {
        StoreOperationContext {
            operation_id: operation_id.to_owned(),
            idempotency_key: None,
            correlation_id: operation_id.to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        }
    }

    /// The live phase of one resource: the manager view for converted rows,
    /// the durable row's `/status/phase` for unconverted rows.
    async fn live_phase(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<&'static str>, SharedProviderEffectError> {
        if route_resource_type(target.resource_type().as_str()) == PlaneRoute::NewPlane {
            let plane = self.plane()?;
            let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
            let view = plane
                .client()
                .get(key)
                .await
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            return Ok(view.map(|view| view_phase(&view)));
        }
        let runtime = self.runtime()?;
        match runtime
            .store()
            .get(StoreGetRequest {
                operation: self.operation("shared-provider-phase"),
                zone: self.zone.clone(),
                target: target.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
        {
            Ok(resource) => Ok(serde_json::from_slice::<Value>(&resource.canonical_json)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/status/phase")
                        .and_then(Value::as_str)
                        .map(|phase| match phase {
                            "Ready" => "Ready",
                            "Deleted" => "Deleted",
                            "Failed" => "Failed",
                            _ => "Pending",
                        })
                })),
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => Ok(None),
            Err(_) => Err(SharedProviderEffectError::Unavailable),
        }
    }

    /// The old-shape document of one resource (`spec`, `metadata`, live
    /// `status.phase`), from the manager view or the durable store.
    async fn resource_value(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<Value>, SharedProviderEffectError> {
        if route_resource_type(target.resource_type().as_str()) != PlaneRoute::NewPlane {
            let runtime = self.runtime()?;
            return match runtime
                .store()
                .get(StoreGetRequest {
                    operation: self.operation("shared-provider-read"),
                    zone: self.zone.clone(),
                    target: target.clone(),
                    expected_uid: None,
                    projection: StoreProjection::Full,
                })
                .await
            {
                Ok(resource) => serde_json::from_slice::<Value>(&resource.canonical_json)
                    .map(Some)
                    .map_err(|_| SharedProviderEffectError::InvalidResource),
                Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => Ok(None),
                Err(_) => Err(SharedProviderEffectError::Unavailable),
            };
        }
        let plane = self.plane()?;
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        let view = plane
            .client()
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let Some(view) = view else {
            return Ok(None);
        };
        let spec = serde_json::from_slice::<Value>(&view.spec)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let metadata = if view.metadata.is_empty() {
            json!({})
        } else {
            serde_json::from_slice::<Value>(&view.metadata)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?
        };
        let uid = crate::shared_provider_driver::resource_uid(&view.uid)?;
        let phase = view_phase(&view);
        Ok(Some(json!({
            "spec": spec,
            "metadata": metadata,
            "status": {"phase": phase},
            "uid": uid.as_str(),
            "generation": view.generation,
        })))
    }

    /// Whether one resource is present, not deleting, and live-Ready.
    async fn resource_ready(&self, target: &ResourceRef) -> bool {
        matches!(self.live_phase(target).await, Ok(Some("Ready")))
    }

    /// The old `dependencies_ready` barrier over the row's declared
    /// dependency references.
    async fn dependencies_ready(
        &self,
        kind: SharedProviderKind,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<bool, SharedProviderEffectError> {
        for dependency in crate::shared_provider_driver::declared_dependency_refs(
            kind,
            &request.spec,
            &request.metadata,
        ) {
            if !self.resource_ready(&dependency).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The Provider generation the old shared Runner resolved from the
    /// Provider row; never guessed.
    async fn provider_generation(
        &self,
        kind: SharedProviderKind,
    ) -> Result<ResourceGeneration, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        match runtime
            .store()
            .get(StoreGetRequest {
                operation: self.operation("shared-provider-provider"),
                zone: self.zone.clone(),
                target: provider_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::MetadataOnly,
            })
            .await
        {
            Ok(resource) if resource.generation.get() != 0 => Ok(resource.generation),
            _ => Err(SharedProviderEffectError::Unavailable),
        }
    }

    fn session_generation(
        &self,
        runtime: &ZoneResourceRuntime,
    ) -> Result<ReconnectGeneration, SharedProviderEffectError> {
        runtime
            .controller_session_generation()
            .ok_or(SharedProviderEffectError::Unavailable)
    }

    /// The assignment fence the old Runner's `AssignmentFenceResolver` minted
    /// for one row: identity from the Provider row, the plane's controller
    /// generation, the live controller session, and the fixed Host target.
    async fn assignment_fence(
        &self,
        kind: SharedProviderKind,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<ResourceAssignmentFence, SharedProviderEffectError> {
        Ok(ResourceAssignmentFence {
            resource_uid: request.uid.clone(),
            resource_revision: ZoneRevision::new(request.generation.get()),
            provider_generation: self.provider_generation(kind).await?,
            controller_generation: self.controller_generation,
            controller_role: ResourceRef::parse(kind.controller_ref())
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            target: ResourceRef::parse(HOST_REF)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            session_generation: self.session_generation(runtime)?,
            epoch: ASSIGNMENT_EPOCH,
            scope: ResourceAssignmentScope::Primary,
        })
    }

    /// The old-shape owner-envelope document of one effect request.
    fn envelope(request: &SharedProviderEffectRequest<'_>) -> Value {
        json!({
            "spec": request.spec.clone(),
            "metadata": request.metadata.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

const NETWORK_CONFIG_VOLUME_SCHEMA_ID: &str = d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_ID;
const NETWORK_CONFIG_VOLUME_SCHEMA_VERSION: &str =
    d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_VERSION;
const NETWORK_CONFIG_CONTENT_KIND: &str = d2b_provider_volume_local::NETWORK_CONFIG_CONTENT_KIND;
const NETWORK_CONFIG_FILE_OWNER: &str = d2b_provider_volume_local::NETWORK_CONFIG_FILE_OWNER;
const NETWORK_CONFIG_FILE_MODE: &str = d2b_provider_volume_local::NETWORK_CONFIG_FILE_MODE;

/// The content fence the old effects threaded into every config-Volume
/// projection (assignment and provenance only: the identity fields were
/// consumed by the old stored-fence comparison the minted fence replaces).
#[derive(Clone)]
struct NetworkContentFence {
    provenance: NetworkProvenance,
    assignment: ResourceAssignmentFence,
}

/// Network readiness facts the controller's barriers consume (old
/// `SharedRunnerNetworkReadiness`): row presence plus the child's live phase,
/// which is the only status a converted child has (R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkReadiness {
    volume_ready: bool,
    guest_ready: bool,
    attachment_ready: bool,
}

/// The Network child port over the manager-routed surface: every upsert is a
/// `ctx.ensure_child` (F1) and every read is the live row.
struct NetworkChildPort<'a> {
    effects: &'a ProductionSharedProviderEffects,
    request: &'a SharedProviderEffectRequest<'a>,
    owner_ref: ResourceRef,
    uid: ResourceUid,
    fence: NetworkContentFence,
    volume_ref: ResourceRef,
    guest_ref: ResourceRef,
    agent_ref: ResourceRef,
}

impl<'a> NetworkChildPort<'a> {
    fn new(
        effects: &'a ProductionSharedProviderEffects,
        request: &'a SharedProviderEffectRequest<'a>,
        owner_ref: ResourceRef,
        uid: ResourceUid,
        fence: NetworkContentFence,
    ) -> Self {
        let vm_name =
            d2b_provider_network_local::ifname::derive_network_child_name(&uid, "vm");
        let agent_name =
            d2b_provider_network_local::ifname::derive_network_child_name(&uid, "agent");
        Self {
            effects,
            request,
            owner_ref,
            uid,
            fence,
            volume_ref: child_ref("Volume", "net-config"),
            guest_ref: child_ref("Guest", &vm_name),
            agent_ref: child_ref("Process", &agent_name),
        }
    }

    async fn current(&self, target: &ResourceRef) -> Result<Option<Value>, NetworkEffectError> {
        self.effects
            .resource_value(target)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    async fn upsert(&self, target: &ResourceRef, spec: Value) -> Result<(), NetworkEffectError> {
        let metadata = json!({
            "ownerRef": self.owner_ref.to_canonical_string(),
            "labels": {},
            "annotations": {},
        });
        self.request
            .children
            .ensure(ChildEnsure {
                type_name: ResourceTypeName::new(target.resource_type().as_str()),
                name: target.name().as_str().to_owned(),
                spec: serde_json::to_vec(&spec).map_err(|_| NetworkEffectError::ConfigVolume)?,
                metadata: serde_json::to_vec(&metadata)
                    .map_err(|_| NetworkEffectError::ConfigVolume)?,
            })
            .await
            .map(|_| ())
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    async fn delete(&self, target: &ResourceRef) -> Result<(), NetworkEffectError> {
        let key = ResourceKey::new(
            self.effects.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        self.request
            .children
            .delete(&key)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    /// The controller's readiness barriers over the live child rows.
    async fn readiness(&self) -> Result<NetworkReadiness, NetworkEffectError> {
        let volume = self.current(&self.volume_ref).await?;
        let guest = self.current(&self.guest_ref).await?;
        let volume_phase = self
            .effects
            .live_phase(&self.volume_ref)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)?;
        let guest_phase = self
            .effects
            .live_phase(&self.guest_ref)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)?;
        let volume_ready = volume_phase == Some("Ready")
            && volume
                .as_ref()
                .is_some_and(|value| network_config_projection_present(value, &self.uid));
        let guest_ready = guest_phase == Some("Ready") && guest.is_some();
        let attachment_ready = volume_phase == Some("Ready")
            && volume.as_ref().is_some_and(|value| {
                value
                    .pointer("/spec/attachments")
                    .and_then(Value::as_array)
                    .is_some_and(|attachments| {
                        attachments.iter().any(|attachment| {
                            attachment.get("executionRef").and_then(Value::as_str)
                                == Some(self.guest_ref.to_canonical_string().as_str())
                        })
                    })
            });
        Ok(NetworkReadiness {
            volume_ready,
            guest_ready,
            attachment_ready,
        })
    }
}

/// The row's live phase: the canonical wire phase of the row's observed
/// status (issue #515). `ResourceStatus::wire_phase` owns the vocabulary
/// (`Deleting` renders as the `Deleted` tombstone); an unpublished or stale
/// status is not observed state of the current row and reads `Pending`.
fn view_phase(view: &ResourceView) -> &'static str {
    view.observed_status()
        .as_ref()
        .map(ResourceStatus::wire_phase)
        .unwrap_or("Pending")
}

fn child_ref(resource_type: &str, name: &str) -> ResourceRef {
    ResourceRef::parse(&format!("{resource_type}/{name}"))
        .expect("derived Network child ref is canonical")
}

/// The spec-level content projection check (old
/// `network_config_content_projection_ready` without its persisted-status
/// terms: the materialization evidence lived in `status.resource`, which R11
/// deletes; the Volume actor's live Ready phase is the equivalent gate).
fn network_config_projection_present(value: &Value, volume_uid: &ResourceUid) -> bool {
    let Some(provider) = value.pointer("/spec/provider") else {
        return false;
    };
    let Some(spec) = value.get("spec") else {
        return false;
    };
    if validate_network_config_volume_spec(spec).is_err() {
        return false;
    }
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/content")
            .and_then(|content| {
                d2b_provider_volume_local::NetworkConfigContentProjection::from_settings(content)
                    .ok()
            })
            .is_some_and(|content| content.volume_uid() == volume_uid)
}

fn validate_network_config_volume_spec(spec: &Value) -> Result<(), NetworkEffectError> {
    let provider_ref = spec
        .get("providerRef")
        .and_then(Value::as_str)
        .ok_or(NetworkEffectError::ConfigVolume)?;
    if provider_ref != "Provider/volume-local" {
        return Err(NetworkEffectError::NetworkAdmissionMismatch);
    }
    let mut base = spec.clone();
    if let Some(base) = base.as_object_mut() {
        base.remove("providerRef");
        base.remove("updatePolicy");
        base.remove("provider");
    }
    let volume: VolumeSpec =
        serde_json::from_value(base).map_err(|_| NetworkEffectError::ConfigVolume)?;
    let required = [
        "dnsmasq.conf",
        "nftables.rules",
        "routing.conf",
        "attachments.json",
    ];
    if !required.iter().all(|path| {
        volume.layout().iter().any(|entry| {
            entry.path() == *path
                && entry.entry_type() == d2b_contracts_resource::v3::volume::EntryType::File
                && entry.owner_ref().to_canonical_string() == NETWORK_CONFIG_FILE_OWNER
                && entry.group_ref().to_canonical_string() == NETWORK_CONFIG_FILE_OWNER
                && entry.mode() == NETWORK_CONFIG_FILE_MODE
        })
    }) {
        return Err(NetworkEffectError::ConfigVolume);
    }
    Ok(())
}

fn network_config_provider_matches(
    provider: &Value,
    volume_uid: &ResourceUid,
    owner_ref: &ResourceRef,
    marker: &str,
) -> bool {
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/content")
            .and_then(|content| {
                d2b_provider_volume_local::NetworkConfigContentProjection::from_settings(content)
                    .ok()
            })
            .is_some_and(|content| {
                content.volume_uid() == volume_uid
                    && content.network_ref() == owner_ref
                    && content.ownership_marker() == marker
            })
}

fn network_config_legacy_provider_matches(
    provider: &Value,
    owner_ref: &ResourceRef,
    marker: &str,
) -> bool {
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/ownershipMarker")
            .and_then(Value::as_str)
            == Some(marker)
        && provider
            .pointer("/settings/networkRef")
            .and_then(Value::as_str)
            == Some(owner_ref.to_canonical_string().as_str())
        && provider
            .pointer("/settings/fileOwner")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_OWNER)
        && provider
            .pointer("/settings/fileGroup")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_OWNER)
        && provider
            .pointer("/settings/fileMode")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_MODE)
        && provider.pointer("/settings/files").is_some()
}

fn network_config_spec_with_content(
    mut spec: Value,
    volume_uid: &ResourceUid,
    content: &d2b_provider_network_local::controller::NetworkConfigContent,
    fence: &NetworkContentFence,
    owner_ref: &ResourceRef,
) -> Result<Value, NetworkEffectError> {
    validate_network_config_volume_spec(&spec)?;
    let marker = d2b_contracts_resource::v3::derive_network_ownership_marker(
        &fence.provenance,
        "network-config",
    );
    if spec.get("provider").is_some_and(|provider| {
        !network_config_provider_matches(provider, volume_uid, owner_ref, &marker)
            && !network_config_legacy_provider_matches(provider, owner_ref, &marker)
    }) {
        return Err(NetworkEffectError::NetworkAdmissionMismatch);
    }
    let provider = network_config_provider_extension(volume_uid, content, owner_ref, fence, &marker)?;
    spec.as_object_mut()
        .ok_or(NetworkEffectError::ConfigVolume)?
        .insert("provider".to_owned(), provider);
    Ok(spec)
}

fn network_config_provider_extension(
    volume_uid: &ResourceUid,
    content: &d2b_provider_network_local::controller::NetworkConfigContent,
    owner_ref: &ResourceRef,
    fence: &NetworkContentFence,
    marker: &str,
) -> Result<Value, NetworkEffectError> {
    let file_owner =
        ResourceRef::parse(NETWORK_CONFIG_FILE_OWNER).map_err(|_| NetworkEffectError::ConfigVolume)?;
    let projection = d2b_provider_volume_local::NetworkConfigContentProjection::new(
        volume_uid.clone(),
        owner_ref.clone(),
        fence.provenance.clone(),
        marker,
        file_owner.clone(),
        file_owner,
        NETWORK_CONFIG_FILE_MODE,
        content.dnsmasq.clone(),
        content.nftables.clone(),
        content.routing.clone(),
        content.attachments.clone(),
        content.digest(),
    )
    .map_err(|_| NetworkEffectError::NetworkAdmissionMismatch)?;
    let content = serde_json::to_value(&projection).map_err(|_| NetworkEffectError::ConfigVolume)?;
    Ok(json!({
        "schemaId": NETWORK_CONFIG_VOLUME_SCHEMA_ID,
        "schemaVersion": NETWORK_CONFIG_VOLUME_SCHEMA_VERSION,
        "settings": {
            "kind": NETWORK_CONFIG_CONTENT_KIND,
            "content": content,
            "assignmentFence": {
                "resourceUid": fence.assignment.resource_uid,
                "resourceRevision": fence.assignment.resource_revision,
                "providerGeneration": fence.assignment.provider_generation,
                "controllerGeneration": fence.assignment.controller_generation,
                "controllerRole": fence.assignment.controller_role,
                "target": fence.assignment.target,
                "sessionGeneration": fence.assignment.session_generation,
                "epoch": fence.assignment.epoch,
                "scope": "primary",
            },
        },
    }))
}

impl NetworkResourcePort for NetworkChildPort<'_> {
    async fn upsert_volume_backing(&self, spec: &VolumeSpec) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        if let Some(current) = self.current(&self.volume_ref).await?
            && let Some(provider) = current.pointer("/spec/provider")
        {
            value
                .as_object_mut()
                .ok_or(NetworkEffectError::ConfigVolume)?
                .insert("provider".to_owned(), provider.clone());
        }
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/volume-local".to_owned()),
            );
        self.upsert(&self.volume_ref, value).await
    }

    async fn upsert_volume_content(
        &self,
        content: &d2b_provider_network_local::controller::NetworkConfigContent,
    ) -> Result<(), NetworkEffectError> {
        if content.provenance() != Some(&self.fence.provenance) {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        let current = self
            .current(&self.volume_ref)
            .await?
            .ok_or(NetworkEffectError::ConfigVolume)?;
        if current.pointer("/metadata/ownerRef").and_then(Value::as_str)
            != Some(self.owner_ref.to_canonical_string().as_str())
            || current.pointer("/spec/providerRef").and_then(Value::as_str)
                != Some("Provider/volume-local")
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        validate_network_config_volume_spec(&spec)?;
        let volume_uid = current
            .pointer("/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(NetworkEffectError::ConfigVolume)?;
        spec = network_config_spec_with_content(spec, &volume_uid, content, &self.fence, &self.owner_ref)?;
        self.upsert(&self.volume_ref, spec).await
    }

    async fn upsert_guest(
        &self,
        spec: &d2b_contracts_resource::v3::guest::GuestSpec,
    ) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/runtime-cloud-hypervisor".to_owned()),
            );
        self.upsert(&self.guest_ref, value).await
    }

    async fn attach_volume(
        &self,
        attachment: &d2b_contracts_resource::v3::volume::VolumeAttachment,
    ) -> Result<(), NetworkEffectError> {
        let current = self
            .current(&self.volume_ref)
            .await?
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let attachments = spec
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .entry("attachments")
            .or_insert_with(|| Value::Array(Vec::new()));
        let attachments = attachments
            .as_array_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let attachment =
            serde_json::to_value(attachment).map_err(|_| NetworkEffectError::ConfigVolume)?;
        if !attachments.iter().any(|current| current == &attachment) {
            attachments.push(attachment);
        }
        self.upsert(&self.volume_ref, spec).await
    }

    async fn upsert_agent(
        &self,
        spec: &d2b_contracts_resource::v3::process::ProcessSpec,
    ) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/system-minijail".to_owned()),
            );
        self.upsert(&self.agent_ref, value).await
    }

    async fn reconcile_mdns(&self, enabled: bool) -> Result<(), NetworkEffectError> {
        if enabled {
            return Err(NetworkEffectError::ConfigVolume);
        }
        Ok(())
    }

    async fn delete_processes(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.agent_ref).await
    }

    async fn detach_volume(&self) -> Result<(), NetworkEffectError> {
        let Some(current) = self.current(&self.volume_ref).await? else {
            return Ok(());
        };
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        if let Some(attachments) = spec
            .as_object_mut()
            .and_then(|spec| spec.get_mut("attachments"))
            .and_then(Value::as_array_mut)
        {
            attachments.retain(|attachment| {
                attachment.get("executionRef").and_then(Value::as_str)
                    != Some(self.guest_ref.to_canonical_string().as_str())
            });
        }
        self.upsert(&self.volume_ref, spec).await
    }

    async fn delete_guest(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.guest_ref).await
    }

    async fn delete_volume(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.volume_ref).await
    }
}

impl ProductionSharedProviderEffects {
    /// Old `network_admission`: root-owned admission evidence for one
    /// Network row.
    async fn network_admission(
        &self,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
        spec: &d2b_contracts_resource::v3::network::NetworkSpec,
        resolver: &d2b_core::bundle_resolver::BundleResolver,
    ) -> Result<NetworkAdmissionProof, SharedProviderEffectError> {
        let zone_uid = runtime
            .authority_zone_uid()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let network_generation = request.generation;
        let network_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
        let mut guest_uids = Vec::new();
        let mut attachment_generation = network_generation.get();
        for attachment in spec.attachments() {
            let attached = self
                .resource_value(attachment.execution_ref())
                .await?
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            if attached.pointer("/metadata/zone").and_then(Value::as_str)
                != Some(request.zone.as_str())
            {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            attachment_generation = attachment_generation.max(
                attached
                    .pointer("/metadata/generation")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
            if attachment.execution_ref().resource_type().as_str() == "Guest" {
                guest_uids.push(
                    attached
                        .pointer("/metadata/uid")
                        .and_then(Value::as_str)
                        .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                );
            }
            let reciprocal = attached
                .pointer("/spec/networkAttachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| {
                    attachments.iter().any(|candidate| {
                        candidate.get("networkRef").and_then(Value::as_str)
                            == Some(network_ref.as_str())
                    })
                });
            if !reciprocal {
                return Err(SharedProviderEffectError::InvalidResource);
            }
        }
        for guest in runtime
            .committed_resources_of_type("Guest")
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?
        {
            let attached = guest
                .pointer("/spec/networkAttachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| {
                    attachments.iter().any(|candidate| {
                        candidate.get("networkRef").and_then(Value::as_str)
                            == Some(network_ref.as_str())
                    })
                });
            if !attached {
                continue;
            }
            if guest.pointer("/metadata/zone").and_then(Value::as_str)
                != Some(request.zone.as_str())
            {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            let guest_uid = guest
                .pointer("/metadata/uid")
                .and_then(Value::as_str)
                .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            guest_uids.push(guest_uid);
            let generation = guest
                .pointer("/metadata/generation")
                .and_then(Value::as_u64)
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            attachment_generation = attachment_generation.max(generation);
        }
        let installed_generation = resolver
            .installed_generation_identity()
            .and_then(|identity| {
                d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(
                    identity.as_str().to_owned(),
                )
                .ok()
            })
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let attachment_generation =
            ResourceGeneration::new(attachment_generation).map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let intent = NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                zone_uid,
                request.uid.clone(),
                network_generation,
                attachment_generation,
                installed_generation,
            ),
            spec.clone(),
            guest_uids,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let plane = self
            .state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.clone())
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let occupancy = observe_host_network().map_err(|_| SharedProviderEffectError::Unavailable)?;
        plane
            .network_admission_index()
            .lock()
            .await
            .admit(intent, &occupancy)
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    /// The Network content fence of one effect call (old
    /// `SharedRunnerNetworkContentFence`).
    async fn network_content_fence(
        &self,
        kind: SharedProviderKind,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
        admission: &NetworkAdmissionProof,
    ) -> Result<NetworkContentFence, SharedProviderEffectError> {
        let assignment = self.assignment_fence(kind, runtime, request).await?;
        let provenance = NetworkProvenance::new(
            admission.key().zone_uid().clone(),
            admission.key().network_uid().clone(),
            admission.key().network_generation(),
            admission.key().attachment_generation(),
            admission.key().bundle_generation().clone(),
        );
        let _ = kind;
        Ok(NetworkContentFence {
            provenance,
            assignment,
        })
    }

    /// The Network reconcile input over the live child rows.
    fn network_input(
        &self,
        spec: &d2b_contracts_resource::v3::network::NetworkSpec,
        request: &SharedProviderEffectRequest<'_>,
        admission: NetworkAdmissionProof,
        readiness: NetworkReadiness,
        attachments: Vec<AttachmentRealization>,
    ) -> ReconcileInput {
        let mdns_enabled = request
            .spec
            .pointer("/mdns/enable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ReconcileInput {
            spec: spec.clone(),
            mdns_enabled,
            network_uid: request.uid.clone(),
            network_generation: request.generation,
            attachment_generation: admission.key().attachment_generation(),
            installed_generation: admission.key().bundle_generation().clone(),
            admission,
            artifact_catalog: vec![ArtifactCatalogEntry::new(
                spec.net_vm_system_artifact_id().clone(),
                ArtifactKind::NixosSystem,
            )],
            user_ready: true,
            host_memory_budget_available:
                d2b_provider_network_local::controller::CONFIG_VOLUME_MAX_BYTES,
            volume_ready: readiness.volume_ready,
            guest_ready: readiness.guest_ready,
            volume_attachment_ready: readiness.attachment_ready,
            workload_fds_closed: true,
            agent_deleted: true,
            mdns_deleted: !mdns_enabled,
            volume_attachment_removed: true,
            guest_deleted: true,
            volume_deleted: true,
            attachments,
        }
    }

    fn network_spec(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<d2b_contracts_resource::v3::network::NetworkSpec, SharedProviderEffectError> {
        let mut spec_value = request
            .spec
            .clone();
        if let Some(spec) = spec_value.as_object_mut() {
            for field in ["providerRef", "updatePolicy", "provider"] {
                spec.remove(field);
            }
        }
        serde_json::from_value(spec_value).map_err(|_| SharedProviderEffectError::InvalidResource)
    }
}

// ---------------------------------------------------------------------------
// TPM
// ---------------------------------------------------------------------------

fn tpm_opaque_bytes(domain: &str, value: &str) -> [u8; 32] {
    let digest = Sha256::digest(format!("{domain}:{value}").as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(&digest);
    bytes
}

fn tpm_state_intent(
    device_uid: &ResourceUid,
    vm_id: &str,
) -> d2b_provider_device_tpm::StateDirIntent {
    d2b_provider_device_tpm::StateDirIntent::new(
        d2b_provider_device_tpm::StateDirectoryToken::from_core(tpm_opaque_bytes(
            "d2b:tpm-state/v1",
            vm_id,
        )),
        d2b_provider_device_tpm::TamperMarkerToken::from_core(tpm_opaque_bytes(
            "d2b:tpm-marker/v1",
            device_uid.as_str(),
        )),
        d2b_provider_device_tpm::StateOwnerToken::from_core(
            tpm_opaque_bytes("d2b:tpm-owner/v1", vm_id)[..16]
                .try_into()
                .expect("fixed owner token length"),
        ),
    )
}

// ---------------------------------------------------------------------------
// USBIP
// ---------------------------------------------------------------------------

/// The production USBIP port over the zone-wide authority ledger (old
/// `SharedRunnerUsbipPort`).
type SharedRunnerUsbipPort<'a> = d2b_provider_device_usbip::ProductionPort<
    crate::usbip_production::DaemonUsbipDispatcher<'a, SharedRunnerUsbipChildren>,
>;

/// Fail-closed child port for the USBIP dispatcher: production realizes the
/// attach path through the broker, never through child rows.
struct SharedRunnerUsbipChildren;

impl UsbipChildResourcePort for SharedRunnerUsbipChildren {
    fn ensure_attach_process(
        &mut self,
        _binding: &d2b_provider_device_usbip::BindingIdentity,
        _proxy: &d2b_provider_device_usbip::BindingProxyLease,
    ) -> Result<
        d2b_provider_device_usbip::AttachProcessIdentity,
        d2b_provider_device_usbip::BindingLifecycleError,
    > {
        Err(d2b_provider_device_usbip::BindingLifecycleError::Transient)
    }

    fn observe_attach_process(
        &mut self,
        _binding: &d2b_provider_device_usbip::BindingIdentity,
        _identity: &d2b_provider_device_usbip::AttachProcessIdentity,
    ) -> Result<
        d2b_provider_device_usbip::AttachmentObservation,
        d2b_provider_device_usbip::BindingLifecycleError,
    > {
        Err(d2b_provider_device_usbip::BindingLifecycleError::Transient)
    }

    fn delete_guest_endpoint(
        &mut self,
        _binding: &d2b_provider_device_usbip::BindingIdentity,
        _proxy: &d2b_provider_device_usbip::BindingProxyLease,
    ) -> Result<(), d2b_provider_device_usbip::BindingLifecycleError> {
        Err(d2b_provider_device_usbip::BindingLifecycleError::Transient)
    }

    fn delete_attach_process(
        &mut self,
        _binding: &d2b_provider_device_usbip::BindingIdentity,
        _identity: &d2b_provider_device_usbip::AttachProcessIdentity,
    ) -> Result<(), d2b_provider_device_usbip::BindingLifecycleError> {
        Err(d2b_provider_device_usbip::BindingLifecycleError::Transient)
    }
}

impl ProductionSharedProviderEffects {
    /// Old `usbip_service_port`: the broker-backed dispatcher for one USBIP
    /// Service, over the zone-wide authority ledger.
    async fn usbip_service_port<'a>(
        &'a self,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<
        (ResourceUid, bool, SharedRunnerUsbipPort<'a>),
        SharedProviderEffectError,
    > {
        let zone_uid = runtime
            .authority_zone_uid()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let device_ref = ResourceRef::parse(
            request
                .spec
                .pointer("/backingDeviceRef")
                .and_then(Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let device = self
            .resource_value(&device_ref)
            .await?
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        if device.pointer("/spec/providerRef").and_then(Value::as_str)
            != Some(d2b_provider_device_usbip::PROVIDER_REF)
            || device.pointer("/status/phase").and_then(Value::as_str) != Some("Ready")
        {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let device_uid = device
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let env = request
            .spec
            .pointer("/env")
            .and_then(Value::as_str)
            .unwrap_or(request.target.name.as_str());
        let physical_key =
            d2b_core::device_usbip_adapter::UsbipCoreAdapter::physical_usb_backing_key(
                device_uid.as_str().as_bytes(),
            )
            .as_bytes();
        let binding_context = crate::usbip_production::UsbipBindingContext::new(
            request.target.name.as_str(),
            env,
            format!("shared-usbip-bind-{}", request.uid.as_str()),
            format!("shared-usbip-runner-{}", request.uid.as_str()),
            physical_key,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let port = crate::usbip_production::DaemonUsbipDispatcher::new(
            &self.state,
            binding_context,
            Arc::clone(&self.usbip_ledger),
            SharedRunnerUsbipChildren,
        )
        .into_port();
        let opted_in = request.spec.pointer("/mode").and_then(Value::as_str) == Some("authority");
        Ok((zone_uid, opted_in, port))
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

struct DaemonGpuLifecyclePort<'a> {
    /// Per-resource Provider state owned by the calling driver (old
    /// zone-wide maps, now per-resource).
    state_maps: &'a SharedProviderResourceState,
    state: Arc<ServerState>,
    runtime: Arc<ZoneResourceRuntime>,
    resolver: d2b_core::bundle_resolver::BundleResolver,
    device_ref: ResourceRef,
    device_uid: ResourceUid,
    holder_ref: ResourceRef,
    generation: ResourceGeneration,
    settings: d2b_provider_device_gpu::GpuSettings,
    operation_id: String,
    opened_devices: Vec<OwnedFd>,
}

impl<'a> DaemonGpuLifecyclePort<'a> {
    fn role_key(role: d2b_provider_device_gpu::GpuProcessRole) -> u8 {
        match role {
            d2b_provider_device_gpu::GpuProcessRole::FullGpu => 0,
            d2b_provider_device_gpu::GpuProcessRole::RenderNode => 1,
            d2b_provider_device_gpu::GpuProcessRole::Video => 2,
        }
    }

    fn intent(
        &self,
        template: &str,
    ) -> Result<d2b_core::bundle_resolver::ResolvedRunnerIntent, d2b_provider_device_gpu::GpuEffectError>
    {
        let vm = self.holder_ref.name().as_str();
        self.resolver
            .find_runner_intent_for_process_in_vm(
                Some(vm),
                "Host/host-system",
                d2b_core::processes::ProcessExecutionDomain::System,
                None,
                template,
            )
            .cloned()
            .ok_or(d2b_provider_device_gpu::GpuEffectError::SpawnRejected)
    }

    fn open_device_classes(
        &mut self,
        role_id: &str,
        classes: &[&str],
    ) -> Result<(), d2b_provider_device_gpu::GpuEffectError> {
        for device_class in classes {
            let request = d2b_contracts_broker::broker_wire::BrokerRequest::OpenDevice(
                d2b_contracts_broker::broker_wire::OpenDeviceRequest {
                    role_id: d2b_contracts::types::RoleId::new(role_id.to_owned()),
                    device_class: (*device_class).to_owned(),
                    tracing_span_id: None,
                },
            );
            let (response, fds) = crate::dispatch_broker_request_with_fds_timeout_as(
                &self.state,
                request,
                BrokerCallerRole::AdminUid {
                    uid: self.state.daemon_uid,
                },
                std::time::Duration::from_secs(10),
            )
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::OpenRejected)?;
            let accepted = matches!(
                response,
                d2b_contracts_broker::broker_wire::BrokerResponse::Ack(response)
                    if response.accepted
            );
            if !accepted || fds.len() != 1 {
                crate::close_received_fds(&fds);
                return Err(d2b_provider_device_gpu::GpuEffectError::OpenRejected);
            }
            let fd = crate::duplicate_received_fd(&fds, 0, "GPU device grant")
                .map_err(|_| d2b_provider_device_gpu::GpuEffectError::OpenRejected)?;
            crate::close_received_fds(&fds);
            self.opened_devices.push(fd);
        }
        Ok(())
    }

    fn spawn_worker(
        &mut self,
        template: &str,
        process_name: &str,
        principal: &d2b_provider_device_gpu::GpuPrincipalToken,
        platform: &d2b_provider_device_gpu::GpuPlatformToken,
        generation: ResourceGeneration,
        role: d2b_contracts_broker::broker_wire::RunnerRole,
    ) -> Result<
        d2b_provider_device_gpu::GpuProcessIdentity,
        d2b_provider_device_gpu::GpuEffectError,
    > {
        let intent = self.intent(template)?;
        let execution_ref = ResourceRef::parse(&intent.execution_ref)
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?;
        let owner_uid = crate::block_on_future(self.runtime.committed_resource_value(
            &self.holder_ref,
            &self.operation_id,
        ))
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)
        .and_then(|value| {
            value
                .pointer("/metadata/uid")
                .and_then(Value::as_str)
                .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                .ok_or(d2b_provider_device_gpu::GpuEffectError::SpawnRejected)
        })?;
        let resource_ref = ResourceRef::parse(&format!("Process/{process_name}"))
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?;
        let request = d2b_contracts_broker::broker_wire::BrokerRequest::SpawnRunner(
            d2b_contracts_broker::broker_wire::SpawnRunnerRequest {
                vm_id: VmId::new(self.holder_ref.name().as_str()),
                role_id: d2b_contracts::types::RoleId::new(intent.role_id.clone()),
                resource_ref: Some(resource_ref.clone()),
                resource_uid: None,
                zone_uid: self.runtime.authority_zone_uid().cloned(),
                owner_ref: Some(self.holder_ref.clone()),
                owner_uid: Some(owner_uid),
                provider_ref: Some(
                    ResourceRef::parse("Provider/system-minijail")
                        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?,
                ),
                bundle_content_identity: self
                    .runtime
                    .authority_bundle_generation()
                    .map(|value| value.as_str().to_owned()),
                provider_identity: None,
                template_identity: None,
                generation: Some(generation.get()),
                runtime_scope: Some(Self::scope_digest(
                    &self.device_uid,
                    &self.operation_id,
                )),
                activation_input: None,
                sandbox_plan: None,
                role,
                bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
                execution_ref: Some(execution_ref),
                execution_domain: Some(match intent.execution_domain {
                    d2b_core::processes::ProcessExecutionDomain::System => {
                        d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System
                    }
                    d2b_core::processes::ProcessExecutionDomain::User => {
                        d2b_contracts_resource::v3::execution_policy::ExecutionDomain::User
                    }
                }),
                user_ref: intent
                    .user_ref
                    .as_deref()
                    .and_then(|value| ResourceRef::parse(value).ok()),
                guest_execution: None,
                runtime_allocations: Vec::new(),
                tracing_span_id: None,
                workload_identity: None,
                inherited_fd_count: u16::try_from(self.opened_devices.len())
                    .map_err(|_| d2b_provider_device_gpu::GpuEffectError::OpenRejected)?,
                network_tap_context: None,
                            launch_args: None,
            },
        );
        let request_fds = self
            .opened_devices
            .iter()
            .map(AsRawFd::as_raw_fd)
            .collect::<Vec<_>>();
        let (response, received_fds) = crate::dispatch_broker_request_with_optional_request_fds(
            &self.state,
            request,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            &request_fds,
            std::time::Duration::from_secs(30),
        )
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?;
        let response = match response {
            d2b_contracts_broker::broker_wire::BrokerResponse::SpawnRunner(response) => response,
            _ => {
                crate::close_received_fds(&received_fds);
                return Err(d2b_provider_device_gpu::GpuEffectError::SpawnRejected);
            }
        };
        if response.vm_id != VmId::new(self.holder_ref.name().as_str())
            || response.role != role
            || response.role_id.as_str() != intent.role_id
            || response.zone_uid != self.runtime.authority_zone_uid().cloned()
            || response.owner_ref.as_ref() != Some(&self.holder_ref)
            || response.generation != Some(generation.get())
            || response.resource_ref.as_ref() != Some(&resource_ref)
            || response.pid <= 0
        {
            crate::close_received_fds(&received_fds);
            return Err(d2b_provider_device_gpu::GpuEffectError::StaleDeviceIdentity);
        }
        let pidfd = crate::duplicate_received_fd(
            &received_fds,
            response.pidfd_index,
            "GPU SpawnRunner pidfd",
        )
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?;
        crate::close_received_fds(&received_fds);
        let vm = self.holder_ref.name().as_str().to_owned();
        if self
            .state
            .pidfd_table
            .register(
                vm.clone(),
                intent.role_id.clone(),
                crate::PidfdEntry {
                    pidfd,
                    pid: response.pid,
                    start_time_ticks: response.start_time_ticks,
                },
            )
            .is_err()
        {
            tracing::debug!(
                vm = %vm,
                role = %intent.role_id,
                "GPU pidfd registration failed; rejecting spawn and stopping VM",
            );
            let _ = crate::stop_vm_pidfd_role(
                &self.state,
                BrokerCallerRole::AdminUid {
                    uid: self.state.daemon_uid,
                },
                "device-gpu",
                &vm,
                &intent.role_id,
                std::time::Duration::from_secs(5),
                std::time::Duration::from_secs(5),
            );
            return Err(d2b_provider_device_gpu::GpuEffectError::SpawnRejected);
        }
        if self.state.pidfd_table.snapshot().is_err() {
            self.state.pidfd_table.deregister_if_matches(
                &vm,
                &intent.role_id,
                response.pid,
                response.start_time_ticks,
            );
            tracing::debug!(
                vm = %vm,
                role = %intent.role_id,
                "GPU pidfd snapshot failed; rejecting spawn and stopping VM",
            );
            let _ = crate::stop_vm_pidfd_role(
                &self.state,
                BrokerCallerRole::AdminUid {
                    uid: self.state.daemon_uid,
                },
                "device-gpu",
                &vm,
                &intent.role_id,
                std::time::Duration::from_secs(5),
                std::time::Duration::from_secs(5),
            );
            return Err(d2b_provider_device_gpu::GpuEffectError::SpawnRejected);
        }
        let identity = d2b_provider_device_gpu::GpuProcessIdentity::from_core(
            Self::process_digest(&intent.intent_id, response.pid, response.start_time_ticks),
            match role {
                d2b_contracts_broker::broker_wire::RunnerRole::Video => {
                    d2b_provider_device_gpu::GpuProcessRole::Video
                }
                _ if self.settings.render_node_only => {
                    d2b_provider_device_gpu::GpuProcessRole::RenderNode
                }
                _ => d2b_provider_device_gpu::GpuProcessRole::FullGpu,
            },
            principal.clone(),
            platform.clone(),
            generation,
        );
        self.state_maps.gpu_processes
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::SpawnRejected)?
            .insert(
                (self.device_uid.clone(), Self::role_key(identity.role())),
                identity.clone(),
            );
        Ok(identity)
    }

    fn scope_digest(device_uid: &ResourceUid, operation_id: &str) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b:gpu-runtime-scope/v1");
        digest.update(device_uid.as_str().as_bytes());
        digest.update(operation_id.as_bytes());
        digest.finalize().into()
    }

    fn process_digest(intent_id: &str, pid: i32, start_time_ticks: u64) -> [u8; 16] {
        let mut digest = Sha256::new();
        digest.update(b"d2b:gpu-process/v1");
        digest.update(intent_id.as_bytes());
        digest.update(pid.to_be_bytes());
        digest.update(start_time_ticks.to_be_bytes());
        let digest: [u8; 32] = digest.finalize().into();
        digest[..16].try_into().expect("fixed process token length")
    }
}

impl d2b_provider_device_gpu::GpuLifecycleEffectPort for DaemonGpuLifecyclePort<'_> {
    fn reserve_authority(
        &mut self,
        admission: &d2b_provider_device_gpu::GpuAuthorityAdmission,
    ) -> Result<d2b_provider_device_gpu::GpuAuthorityLease, d2b_provider_device_gpu::GpuEffectError>
    {
        if admission.owner().device_uid() != &self.device_uid
            || admission.owner().holder_ref() != &self.holder_ref
            || admission.owner().generation() != self.generation
        {
            return Err(d2b_provider_device_gpu::GpuEffectError::StaleDeviceIdentity);
        }
        let request = AuthorityRequest::gpu_from_core(
            admission.owner().host_uid().clone(),
            self.device_ref.clone(),
            admission.owner().device_uid().clone(),
            admission.owner().generation(),
            *admission.backing().as_bytes(),
            admission.render_node_only(),
            admission.max_holders() as usize,
        )
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?;
        let lease = crate::block_on_future(async {
            self.runtime
                .authority_index()
                .lock()
                .await
                .admit_authority(request)
        })
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?;
        let token = lease.token_bytes();
        self.state_maps.gpu_authority_leases
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?
            .insert(token, lease);
        Ok(d2b_provider_device_gpu::GpuAuthorityLease::from_core(token))
    }

    fn open_authorized_devices(
        &mut self,
        admission: &d2b_provider_device_gpu::GpuAuthorityAdmission,
        tokens: &d2b_provider_device_gpu::GpuEffectTokenSet,
    ) -> Result<d2b_provider_device_gpu::GpuLaunchTicket, d2b_provider_device_gpu::GpuEffectError>
    {
        if admission.owner().device_uid() != &self.device_uid
            || admission.owner().generation() != self.generation
            || !admission.owner().holder_ref().eq(&self.holder_ref)
        {
            return Err(d2b_provider_device_gpu::GpuEffectError::StaleDeviceIdentity);
        }
        if tokens.is_empty() {
            return Err(d2b_provider_device_gpu::GpuEffectError::StaleDeviceIdentity);
        }
        let gpu_intent = self.intent(if self.settings.render_node_only {
            "render-node-worker"
        } else {
            "gpu-worker"
        })?;
        let mut classes = if self.settings.render_node_only {
            vec!["dri"]
        } else {
            vec!["kvm", "dri", "udmabuf"]
        };
        self.open_device_classes(&gpu_intent.role_id, &classes)?;
        if self.settings.video_sidecar {
            let video_intent = self.intent("video-worker")?;
            classes.clear();
            classes.push("dri");
            if self.settings.video_nvidia_decode {
                classes.extend(["nvidia-ctl", "nvidia-device", "nvidia-uvm"]);
            }
            self.open_device_classes(&video_intent.role_id, &classes)?;
        }
        Ok(d2b_provider_device_gpu::GpuLaunchTicket::from_core(
            Self::scope_digest(&self.device_uid, &self.operation_id)[..16]
                .try_into()
                .expect("fixed launch ticket length"),
        ))
    }

    fn start_gpu_worker(
        &mut self,
        spec: &d2b_provider_device_gpu::GpuWorkerSpec,
        _ticket: &d2b_provider_device_gpu::GpuLaunchTicket,
        principal: &d2b_provider_device_gpu::GpuPrincipalToken,
        platform: &d2b_provider_device_gpu::GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<
        d2b_provider_device_gpu::GpuProcessIdentity,
        d2b_provider_device_gpu::GpuEffectError,
    > {
        self.spawn_worker(
            spec.template(),
            &format!("gpu-{}", self.device_ref.name().as_str()),
            principal,
            platform,
            generation,
            d2b_contracts_broker::broker_wire::RunnerRole::Gpu,
        )
    }

    fn start_video_worker(
        &mut self,
        spec: &d2b_provider_device_gpu::VideoWorkerSpec,
        _ticket: &d2b_provider_device_gpu::GpuLaunchTicket,
        principal: &d2b_provider_device_gpu::GpuPrincipalToken,
        platform: &d2b_provider_device_gpu::GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<
        d2b_provider_device_gpu::GpuProcessIdentity,
        d2b_provider_device_gpu::GpuEffectError,
    > {
        self.spawn_worker(
            spec.template(),
            &format!("video-{}", self.device_ref.name().as_str()),
            principal,
            platform,
            generation,
            d2b_contracts_broker::broker_wire::RunnerRole::Video,
        )
    }

    fn observe_worker(
        &mut self,
        identity: &d2b_provider_device_gpu::GpuProcessIdentity,
    ) -> Result<
        d2b_provider_device_gpu::GpuProcessObservation,
        d2b_provider_device_gpu::GpuEffectError,
    > {
        let known = self.state_maps
            .gpu_processes
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::ProcessObservationUnavailable)?
            .get(&(self.device_uid.clone(), Self::role_key(identity.role())))
            .is_some_and(|current| current == identity);
        if !known {
            return Ok(d2b_provider_device_gpu::GpuProcessObservation::Missing);
        }
        let intent = self.intent(match identity.role() {
            d2b_provider_device_gpu::GpuProcessRole::Video => "video-worker",
            d2b_provider_device_gpu::GpuProcessRole::RenderNode => "render-node-worker",
            d2b_provider_device_gpu::GpuProcessRole::FullGpu => "gpu-worker",
        })?;
        if self
            .state
            .pidfd_table
            .contains(self.holder_ref.name().as_str(), &intent.role_id)
        {
            Ok(d2b_provider_device_gpu::GpuProcessObservation::Matching(
                identity.clone(),
            ))
        } else {
            Ok(d2b_provider_device_gpu::GpuProcessObservation::Missing)
        }
    }

    fn stop_worker(
        &mut self,
        identity: &d2b_provider_device_gpu::GpuProcessIdentity,
    ) -> Result<
        d2b_provider_device_gpu::GpuClosureProof,
        d2b_provider_device_gpu::GpuEffectError,
    > {
        let intent = self.intent(match identity.role() {
            d2b_provider_device_gpu::GpuProcessRole::Video => "video-worker",
            d2b_provider_device_gpu::GpuProcessRole::RenderNode => "render-node-worker",
            d2b_provider_device_gpu::GpuProcessRole::FullGpu => "gpu-worker",
        })?;
        let known = self.state_maps
            .gpu_processes
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::CloseUnconfirmed)?
            .get(&(self.device_uid.clone(), Self::role_key(identity.role())))
            .is_some_and(|current| current == identity);
        if !known {
            return Err(d2b_provider_device_gpu::GpuEffectError::StaleDeviceIdentity);
        }
        crate::stop_vm_pidfd_role(
            &self.state,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            "device-gpu",
            self.holder_ref.name().as_str(),
            &intent.role_id,
            std::time::Duration::from_secs(10),
            std::time::Duration::from_secs(10),
        )
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::CloseUnconfirmed)?;
        self.state_maps.gpu_processes
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::CloseUnconfirmed)?
            .remove(&(self.device_uid.clone(), Self::role_key(identity.role())));
        Ok(d2b_provider_device_gpu::GpuClosureProof::from_core(
            identity.clone(),
        ))
    }

    fn release_authority(
        &mut self,
        lease: d2b_provider_device_gpu::GpuAuthorityLease,
        _closures: &[d2b_provider_device_gpu::GpuClosureProof],
    ) -> Result<(), d2b_provider_device_gpu::GpuEffectError> {
        let token = *lease.as_bytes();
        let generic = self.state_maps
            .gpu_authority_leases
            .lock()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?
            .remove(&token)
            .ok_or(d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?;
        let result = crate::block_on_future(async {
            self.runtime
                .authority_index()
                .lock()
                .await
                .release_authority(&generic)
        });
        if result.is_err() {
            tracing::debug!(
                device = %self.device_ref.to_canonical_string(),
                "GPU authority lease release failed; lease restored",
            );
            self.state_maps.gpu_authority_leases
                .lock()
                .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)?
                .insert(token, generic);
            return Err(d2b_provider_device_gpu::GpuEffectError::AuthorityConflict);
        }
        Ok(())
    }
}



impl ProductionSharedProviderEffects {
    /// One GPU authority digest (old `DaemonSharedProviderEffects::gpu_digest`
    /// with the driver's controller generation in place of the old context).
    fn gpu_digest(
        &self,
        domain: &str,
        request: &SharedProviderEffectRequest<'_>,
        assignment_epoch: u64,
        extra: &str,
    ) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(domain.as_bytes());
        digest.update([0]);
        digest.update(request.uid.as_str().as_bytes());
        digest.update([0]);
        digest.update(request.generation.get().to_be_bytes());
        digest.update(self.controller_generation.get().to_be_bytes());
        digest.update(assignment_epoch.to_be_bytes());
        digest.update(extra.as_bytes());
        digest.finalize().into()
    }

    /// Old `gpu_admission`: the authority-fenced admission for one GPU Device.
    async fn gpu_admission(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<
        (
            Arc<ZoneResourceRuntime>,
            d2b_provider_device_gpu::GpuAuthorityAdmission,
            d2b_provider_device_gpu::GpuEffectTokenSet,
            d2b_provider_device_gpu::GpuSettings,
            ResourceRef,
        ),
        SharedProviderEffectError,
    > {
        let runtime = self.runtime()?;
        if runtime.authority_zone_uid().is_none() {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let holder_ref = request.owner_ref()?;
        if !matches!(holder_ref.resource_type().as_str(), "Guest" | "Host") {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        if self.resource_value(&holder_ref).await?.is_none() {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let hosts = runtime
            .committed_resources_of_type("Host")
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let [host] = hosts.as_slice() else {
            return Err(SharedProviderEffectError::InvalidResource);
        };
        let host_uid = host
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let settings: d2b_provider_device_gpu::GpuSettings =
            match request.spec.pointer("/provider/settings") {
                Some(settings) => serde_json::from_value(settings.clone())
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?,
                None => d2b_provider_device_gpu::GpuSettings::default(),
            };
        let arbitration: d2b_contracts_resource::v3::device::DeviceArbitration =
            serde_json::from_value(
                request
                    .spec
                    .pointer("/arbitration")
                    .cloned()
                    .unwrap_or_else(|| Value::String("exclusive".to_owned())),
            )
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let max_holders = request
            .spec
            .pointer("/maxConcurrentClaims")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1);
        let assignment = self
            .assignment_fence(SharedProviderKind::GpuDevice, &runtime, request)
            .await?;
        let assignment_epoch = assignment.epoch;
        let session_generation = assignment.session_generation.get();
        let mut backing_digest =
            self.gpu_digest("d2b:gpu-backing/v2", request, assignment_epoch, "backing");
        backing_digest[..8].copy_from_slice(&session_generation.to_be_bytes());
        let platform_digest =
            self.gpu_digest("d2b:gpu-platform/v2", request, assignment_epoch, host_uid.as_str());
        let gpu_principal_digest =
            self.gpu_digest("d2b:gpu-principal/v2", request, assignment_epoch, "gpu");
        let owner = d2b_provider_device_gpu::GpuOwnerProof::new(
            ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            holder_ref.clone(),
            request.uid.clone(),
            host_uid,
            request.generation,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let mut admission = d2b_provider_device_gpu::GpuAuthorityAdmission::new(
            owner,
            d2b_provider_device_gpu::GpuBackingToken::from_core(backing_digest),
            d2b_provider_device_gpu::GpuPlatformToken::from_core(platform_digest),
            arbitration,
            u32::try_from(max_holders).map_err(|_| SharedProviderEffectError::InvalidResource)?,
            settings.render_node_only,
            d2b_provider_device_gpu::GpuPrincipalToken::from_core(gpu_principal_digest),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        if settings.video_sidecar {
            admission = admission
                .with_video_principal(d2b_provider_device_gpu::GpuPrincipalToken::from_core(
                    self.gpu_digest("d2b:gpu-principal/v2", request, assignment_epoch, "video"),
                ))
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        }
        let mut token_values = vec!["dri"];
        if !settings.render_node_only {
            token_values.extend(["kvm", "udmabuf"]);
        }
        if settings.video_sidecar && settings.video_nvidia_decode {
            token_values.extend(["nvidia-ctl", "nvidia-device", "nvidia-uvm"]);
        }
        let tokens = d2b_provider_device_gpu::GpuEffectTokenSet::from_core(
            token_values
                .into_iter()
                .map(|device_class| {
                    d2b_provider_device_gpu::GpuEffectToken::from_core(self.gpu_digest(
                        "d2b:gpu-device-grant/v2",
                        request,
                        assignment_epoch,
                        device_class,
                    ))
                })
                .collect(),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        Ok((runtime, admission, tokens, settings, holder_ref))
    }
}

/// Take the GPU device grants one driver is holding (old
/// `take_gpu_opened_devices`).
fn take_gpu_opened_devices(
    state: &SharedProviderResourceState,
    device_uid: &ResourceUid,
) -> Result<Vec<OwnedFd>, SharedProviderEffectError> {
    Ok(state
        .gpu_opened_devices
        .lock()
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .remove(device_uid)
        .unwrap_or_default())
}

/// Retain the GPU device grants one driver keeps across reconciles (old
/// `retain_gpu_opened_devices`).
fn retain_gpu_opened_devices(
    state: &SharedProviderResourceState,
    device_uid: &ResourceUid,
    opened_devices: Vec<OwnedFd>,
) -> Result<(), SharedProviderEffectError> {
    state
        .gpu_opened_devices
        .lock()
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .insert(device_uid.clone(), opened_devices);
    Ok(())
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

#[async_trait]
impl SharedProviderDriverEffects for ProductionSharedProviderEffects {
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::Network, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let spec = self.network_spec(request)?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let runtime = self.runtime()?;
        let admission = self
            .network_admission(&runtime, request, &spec, &resolver)
            .await?;
        let fence = self
            .network_content_fence(SharedProviderKind::Network, &runtime, request, &admission)
            .await?;
        let owner_ref = crate::shared_provider_driver::key_ref(&request.target).clone();
        let children = NetworkChildPort::new(self, request, owner_ref, request.uid.clone(), fence);
        let readiness = children
            .readiness()
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let broker_context = crate::resolve_network_effect_context(
            &Self::envelope(request),
            &resolver,
            &admission,
        )
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .with_host_global_nic_admission();
        let effects = crate::network_effect_port::production_port(
            &self.state,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            broker_context,
        );
        let input = self.network_input(&spec, request, admission, readiness, Vec::new());
        match NetworkReconciler::new(effects, children)
            .reconcile(&input)
            .await
        {
            Ok(ReconcileProgress::Ready) => Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Ready,
            )),
            Ok(
                ReconcileProgress::Pending(_)
                | ReconcileProgress::Requeue(_)
                | ReconcileProgress::Blocked(_),
            ) => Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            )),
            Err(_) => Err(SharedProviderEffectError::Unavailable),
        }
    }

    async fn reconcile_tpm(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        let execution_ref = request
            .spec
            .pointer("/provider/settings/executionRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .unwrap_or_else(|| ResourceRef::parse(HOST_REF).expect("Host ref"));
        let holder = request.owner_ref()?;
        if holder.resource_type().as_str() != "Guest" {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let runtime = self.runtime()?;
        let vm_id = VmId::new(holder.name().as_str());
        let migration_intent = BundleOpId::new(format!("legacy-swtpm:vm:{}", vm_id.as_str()));
        let decision = runtime
            .tpm_device_is_admitted(
                &request.uid,
                &crate::shared_provider_driver::key_ref(&request.target),
                vm_id.as_str(),
                &request.operation_id,
                None,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let lifecycle = runtime
            .admit_internal_guest_lifecycle(holder.clone(), &request.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let lifecycle_authorization = crate::provider_effects::LifecycleAuthorization::from_lease(
            lifecycle.lease,
            holder.clone(),
            lifecycle.guest_uid,
            lifecycle.guest_generation,
            lifecycle.provider_assignment_generation,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let log_level = request
            .spec
            .pointer("/provider/settings/logLevel")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(20);
        let binary = d2b_provider_device_tpm::SignedBinaryRef::from_core(
            d2b_provider_device_tpm::BinaryKind::Swtpm,
            tpm_opaque_bytes("d2b:tpm-binary/v1", vm_id.as_str()),
        );
        let mut controllers = request
            .state
            .tpm_controllers
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controller = match controllers.remove(&request.uid) {
            Some(controller) => controller,
            None => d2b_provider_device_tpm::TpmResourceController::new(
                request.uid.clone(),
                crate::shared_provider_driver::key_ref(&request.target).clone(),
                execution_ref.clone(),
            )
            .map_err(|_| SharedProviderEffectError::InvalidResource)?,
        };
        let result = crate::tpm_effect_port::reconcile_device_tpm_controller(
            &self.state,
            &resolver,
            vm_id.clone(),
            migration_intent,
            decision,
            crate::tpm_effect_port::AdmittedTpmDevice::new(
                request.uid.clone(),
                crate::shared_provider_driver::key_ref(&request.target).clone(),
                self.zone.as_str(),
                execution_ref,
                lifecycle_authorization,
            ),
            tpm_state_intent(&request.uid, vm_id.as_str()),
            d2b_provider_device_tpm::SwtpmSettings { log_level },
            binary,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            &mut controller,
        )
        .map_err(|error| {
            tracing::debug!(
                error = ?error,
                device = %crate::shared_provider_driver::key_ref(&request.target).to_canonical_string(),
                "TPM device controller reconcile failed",
            );
            SharedProviderEffectError::Unavailable
        });
        match result {
            Ok(outcome) => {
                controllers.insert(request.uid.clone(), controller);
                match outcome {
                    d2b_provider_device_tpm::TpmResourceOutcome::Ready => {
                        Ok(SharedProviderEffectOutcome::phase(
                            SharedProviderEffectPhase::Ready,
                        ))
                    }
                    d2b_provider_device_tpm::TpmResourceOutcome::Retry => {
                        Ok(SharedProviderEffectOutcome::phase(
                            SharedProviderEffectPhase::Pending,
                        ))
                    }
                    d2b_provider_device_tpm::TpmResourceOutcome::Failed
                    | d2b_provider_device_tpm::TpmResourceOutcome::VolumeRetained => {
                        Err(SharedProviderEffectError::Unavailable)
                    }
                }
            }
            Err(error) => {
                controllers.insert(request.uid.clone(), controller);
                Err(error)
            }
        }
    }

    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(
                match component {
                    UsbipComponent::Device => SharedProviderKind::UsbipDevice,
                    UsbipComponent::Service => SharedProviderKind::UsbipService,
                    UsbipComponent::Binding => SharedProviderKind::UsbipBinding,
                },
                request,
            )
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        match component {
            UsbipComponent::Device => {
                let runtime = self.runtime()?;
                let services = runtime
                    .committed_resources_of_type(
                        d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE,
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                let device_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
                let ready = services.iter().any(|service| {
                    service.pointer("/spec/providerRef").and_then(Value::as_str)
                        == Some(d2b_provider_device_usbip::PROVIDER_REF)
                        && service
                            .pointer("/spec/backingDeviceRef")
                            .and_then(Value::as_str)
                            == Some(device_ref.as_str())
                        && service.pointer("/status/phase").and_then(Value::as_str)
                            == Some("Ready")
                });
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
            UsbipComponent::Service => {
                if self
                    .usbip_services
                    .lock()
                    .map_err(|_| SharedProviderEffectError::Unavailable)?
                    .contains(&request.uid)
                {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Ready,
                    ));
                }
                let runtime = self.runtime()?;
                let (zone_uid, zone_opted_in, mut port) =
                    self.usbip_service_port(&runtime, request).await?;
                let mut lifecycle = d2b_provider_device_usbip::ServiceLifecycle::new(
                    zone_uid.clone(),
                    request.uid.clone(),
                );
                lifecycle
                    .activate(zone_opted_in, zone_uid, &mut port)
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                self.usbip_services
                    .lock()
                    .map_err(|_| SharedProviderEffectError::Unavailable)?
                    .insert(request.uid.clone());
                Ok(SharedProviderEffectOutcome::phase(
                    SharedProviderEffectPhase::Ready,
                ))
            }
            UsbipComponent::Binding => {
                let service_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/serviceRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let guest_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/guestRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let runtime = self.runtime()?;
                let zone_uid = runtime
                    .authority_zone_uid()
                    .cloned()
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let service = self
                    .resource_value(&service_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if service.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_usbip::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if service.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let service_uid = service
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let service_generation = service
                    .pointer("/metadata/generation")
                    .and_then(Value::as_u64)
                    .and_then(|value| ResourceGeneration::new(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let guest = self
                    .resource_value(&guest_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if guest.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let guest_uid = guest
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let admission = self
                    .assignment_fence(SharedProviderKind::UsbipBinding, &runtime, request)
                    .await?;
                let admission = d2b_provider_device_usbip::UsbipBindingAdmission::new(
                    zone_uid,
                    request.uid.clone(),
                    service_uid,
                    guest_uid,
                    service_generation,
                    admission.epoch,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let mut controller =
                    d2b_provider_device_usbip::UsbipBindingController::new_admitted(
                        &crate::shared_provider_driver::key_ref(&request.target),
                        &service_ref,
                        &guest_ref,
                        admission,
                    )
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let desired = d2b_provider_device_usbip::binding_child_resources(
                    &crate::shared_provider_driver::key_ref(&request.target),
                    &service_ref,
                    &guest_ref,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let ready = self
                    .child_set_ready(
                        &desired,
                        &request.target,
                        &request.zone,
                    )
                    .await?;
                if ready {
                    controller
                        .observe_children(true)
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                }
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
        }
    }

    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        let kind = match component {
            SecurityKeyComponent::Device => SharedProviderKind::SecurityKeyDevice,
            SecurityKeyComponent::Service => SharedProviderKind::SecurityKeyService,
            SecurityKeyComponent::Binding => SharedProviderKind::SecurityKeyBinding,
        };
        if !self.dependencies_ready(kind, request).await? {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        match component {
            SecurityKeyComponent::Device => {
                let projection = request.status.clone().unwrap_or_else(|| json!({}));
                let admitted = projection.get("devicePresent").and_then(Value::as_bool) == Some(true)
                    && projection.get("fidoConfirmed").and_then(Value::as_bool) == Some(true);
                Ok(SharedProviderEffectOutcome::projection(
                    if admitted {
                        SharedProviderEffectPhase::Ready
                    } else {
                        SharedProviderEffectPhase::Pending
                    },
                    projection,
                ))
            }
            SecurityKeyComponent::Service => {
                let runtime = self.runtime()?;
                let mode = request
                    .spec
                    .pointer("/mode")
                    .and_then(Value::as_str)
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                if mode == "projection" {
                    let endpoint_ref = request
                        .status
                        .as_ref()
                        .and_then(|status| status.get("relayEndpointRef"))
                        .or_else(|| {
                            request
                                .status
                                .as_ref()
                                .and_then(|status| status.get("details"))
                                .and_then(|details| details.get("relayEndpointRef"))
                        })
                        .and_then(Value::as_str)
                        .and_then(|value| ResourceRef::parse(value).ok())
                        .ok_or(SharedProviderEffectError::Unavailable)?;
                    let endpoint = self
                        .resource_value(&endpoint_ref)
                        .await?
                        .ok_or(SharedProviderEffectError::Unavailable)?;
                    return Ok(SharedProviderEffectOutcome::projection(
                        if endpoint.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
                        {
                            SharedProviderEffectPhase::Ready
                        } else {
                            SharedProviderEffectPhase::Pending
                        },
                        request.status.clone().unwrap_or_else(|| json!({})),
                    ));
                }
                let settings = request
                    .spec
                    .pointer("/provider/settings")
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let device_ref = settings
                    .get("deviceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let relay_endpoint_ref = settings
                    .get("relayEndpointRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                if device_ref.resource_type().as_str() != "Device"
                    || relay_endpoint_ref.resource_type().as_str() != "Endpoint"
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                let device = self
                    .resource_value(&device_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if device.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_security_key::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if device.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let device_uid = device
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let relay_process_name =
                    d2b_provider_device_security_key::security_key_process_name(
                        &device_uid,
                        d2b_provider_device_security_key::SecurityKeyProcessRole::HostRelay,
                    )
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let relay_process_ref =
                    ResourceRef::parse(&format!("Process/{relay_process_name}"))
                        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let process = self
                    .resource_value(&relay_process_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let endpoint = self
                    .resource_value(&relay_endpoint_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let ready =
                    process.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
                        && endpoint.pointer("/status/phase").and_then(Value::as_str)
                            == Some("Ready");
                let _ = runtime;
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
            SecurityKeyComponent::Binding => {
                let service_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/serviceRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let target_ref = request
                    .spec
                    .pointer("/target/guestRef")
                    .or_else(|| request.spec.pointer("/guestRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let service = self
                    .resource_value(&service_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if service.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_security_key::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if service.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let guest = self
                    .resource_value(&target_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if guest.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let desired = if let Some(user_ref) = request
                    .spec
                    .pointer("/target/userRef")
                    .or_else(|| request.spec.pointer("/userRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                {
                    d2b_provider_device_security_key::SecurityKeyController::child_resources_for_user(
                        &crate::shared_provider_driver::key_ref(&request.target),
                        &service_ref,
                        &target_ref,
                        &user_ref,
                    )
                } else {
                    d2b_provider_device_security_key::SecurityKeyController::child_resources(
                        &crate::shared_provider_driver::key_ref(&request.target),
                        &service_ref,
                        &target_ref,
                    )
                }
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let ready = self
                    .child_set_ready(&desired, &request.target, &request.zone)
                    .await?;
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
        }
    }

    async fn reconcile_gpu(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::GpuDevice, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let (runtime, admission, tokens, settings, holder_ref) = self.gpu_admission(request).await?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controllers = request
            .state
            .gpu_controllers
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controller = match controllers.remove(&request.uid) {
            Some(controller) => controller,
            None => d2b_provider_device_gpu::GpuController::new_authorized(
                admission.clone(),
                settings.clone(),
                tokens.clone(),
            )
            .map_err(|_| SharedProviderEffectError::InvalidResource)?,
        };
        if controller
            .admission()
            .is_some_and(|current| current != &admission)
        {
            controllers.insert(request.uid.clone(), controller);
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let opened_devices = take_gpu_opened_devices(request.state, &request.uid)?;
        let mut port = DaemonGpuLifecyclePort {
            state: Arc::clone(&self.state),
            runtime,
            resolver,
            device_ref: crate::shared_provider_driver::key_ref(&request.target).clone(),
            device_uid: request.uid.clone(),
            holder_ref,
            generation: request.generation,
            settings,
            operation_id: request.operation_id.clone(),
            state_maps: request.state,
            opened_devices,
        };
        let result = controller
            .reconcile_lifecycle(&mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)
            .map(|outcome| match outcome {
                d2b_provider_device_gpu::GpuReconcileOutcome::Converged => {
                    SharedProviderEffectPhase::Ready
                }
                d2b_provider_device_gpu::GpuReconcileOutcome::Retry => {
                    SharedProviderEffectPhase::Pending
                }
            });
        let opened_devices = std::mem::take(&mut port.opened_devices);
        retain_gpu_opened_devices(request.state, &request.uid, opened_devices)?;
        controllers.insert(request.uid.clone(), controller);
        result.map(SharedProviderEffectOutcome::phase)
    }

    async fn finalize(
        &self,
        kind: SharedProviderKind,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        match kind {
            SharedProviderKind::Network => self.finalize_network(request).await,
            SharedProviderKind::TpmDevice => self.finalize_tpm(request).await,
            SharedProviderKind::UsbipService => self.finalize_usbip_service(request).await,
            SharedProviderKind::UsbipDevice => {
                let device_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
                let runtime = self.runtime()?;
                let children = runtime
                    .committed_resources_of_type(
                        d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE,
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                if children.iter().any(|child| {
                    child
                        .pointer("/spec/backingDeviceRef")
                        .and_then(Value::as_str)
                        == Some(device_ref.as_str())
                }) {
                    return Ok(SharedProviderFinalize::Pending);
                }
                Ok(SharedProviderFinalize::Complete)
            }
            SharedProviderKind::UsbipBinding => {
                let service_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/serviceRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let guest_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/guestRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let mut controller = d2b_provider_device_usbip::UsbipBindingController::new(
                    &crate::shared_provider_driver::key_ref(&request.target),
                    &service_ref,
                    &guest_ref,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                controller.finalize();
                Ok(SharedProviderFinalize::Complete)
            }
            SharedProviderKind::SecurityKeyService => {
                let runtime = self.runtime()?;
                let service_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
                let bindings = runtime
                    .committed_resources_of_type(
                        d2b_provider_device_security_key::SECURITY_KEY_BINDING_RESOURCE_TYPE,
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                if bindings.iter().any(|binding| {
                    binding.pointer("/spec/serviceRef").and_then(Value::as_str)
                        == Some(service_ref.as_str())
                }) {
                    return Ok(SharedProviderFinalize::Pending);
                }
                Ok(SharedProviderFinalize::Complete)
            }
            SharedProviderKind::SecurityKeyDevice => {
                let runtime = self.runtime()?;
                let device_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
                let services = runtime
                    .committed_resources_of_type(
                        d2b_provider_device_security_key::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                if services.iter().any(|service| {
                    service
                        .pointer("/spec/provider/settings/deviceRef")
                        .and_then(Value::as_str)
                        == Some(device_ref.as_str())
                        || service
                            .pointer("/metadata/ownerRef")
                            .and_then(Value::as_str)
                            == Some(device_ref.as_str())
                }) {
                    return Ok(SharedProviderFinalize::Pending);
                }
                Ok(SharedProviderFinalize::Complete)
            }
            SharedProviderKind::SecurityKeyBinding => Ok(SharedProviderFinalize::Complete),
            SharedProviderKind::GpuDevice => self.finalize_gpu(request).await,
        }
    }
}

impl ProductionSharedProviderEffects {
    /// The children of one Provider-declared Binding set, live-Ready?
    async fn child_set_ready(
        &self,
        desired: &d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildSet,
        owner: &ResourceKey,
        zone: &ZoneId,
    ) -> Result<bool, SharedProviderEffectError> {
        for intent in desired.iter() {
            if *intent.owner_ref() != crate::shared_provider_driver::key_ref(owner) || zone.as_str() != self.zone.as_str() {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            if !self.resource_ready(intent.resource_ref()).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn finalize_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let spec = self.network_spec(request)?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let runtime = self.runtime()?;
        let admission = self
            .network_admission(&runtime, request, &spec, &resolver)
            .await?;
        let fence = self
            .network_content_fence(SharedProviderKind::Network, &runtime, request, &admission)
            .await?;
        let owner_ref = crate::shared_provider_driver::key_ref(&request.target).clone();
        let children = NetworkChildPort::new(self, request, owner_ref, request.uid.clone(), fence);
        let volume = children
            .current(&children.volume_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let guest = children
            .current(&children.guest_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let agent = children
            .current(&children.agent_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let volume_phase = self
            .live_phase(&children.volume_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let guest_phase = self
            .live_phase(&children.guest_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let agent_phase = self
            .live_phase(&children.agent_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let volume_attachment_removed = volume.as_ref().is_none_or(|value| {
            value
                .pointer("/spec/attachments")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        });
        let mdns_enabled = request
            .spec
            .pointer("/mdns/enable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if mdns_enabled {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let broker_context = crate::resolve_network_effect_context(
            &Self::envelope(request),
            &resolver,
            &admission,
        )
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .with_host_global_nic_admission();
        let effects = crate::network_effect_port::production_port(
            &self.state,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            broker_context,
        );
        let input = ReconcileInput {
            spec,
            mdns_enabled,
            network_uid: request.uid.clone(),
            network_generation: request.generation,
            attachment_generation: admission.key().attachment_generation(),
            installed_generation: admission.key().bundle_generation().clone(),
            admission,
            artifact_catalog: Vec::new(),
            user_ready: true,
            host_memory_budget_available:
                d2b_provider_network_local::controller::CONFIG_VOLUME_MAX_BYTES,
            volume_ready: volume_phase == Some("Ready"),
            guest_ready: guest_phase == Some("Ready"),
            volume_attachment_ready: volume_phase == Some("Ready"),
            workload_fds_closed: true,
            agent_deleted: agent.is_none() || agent_phase == Some("Deleted"),
            mdns_deleted: true,
            volume_attachment_removed,
            guest_deleted: guest.is_none() || guest_phase == Some("Deleted"),
            volume_deleted: volume.is_none() || volume_phase == Some("Deleted"),
            attachments: Vec::<AttachmentRealization>::new(),
        };
        let stage = NetworkReconciler::new(effects, children)
            .finalize(&input)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if stage != d2b_provider_network_local::controller::FinalizerStage::Complete {
            return Ok(SharedProviderFinalize::Pending);
        }
        let zone_uid = runtime.authority_zone_uid().cloned();
        let plane = self
            .state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.clone());
        if let (Some(zone_uid), Some(plane)) = (zone_uid, plane) {
            plane
                .network_admission_index()
                .lock()
                .await
                .release_owner_after_finalizer(&zone_uid, &request.uid, true);
        }
        Ok(SharedProviderFinalize::Complete)
    }

    async fn finalize_tpm(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let holder = request.owner_ref()?;
        if holder.resource_type().as_str() != "Guest" {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let execution_ref = request
            .spec
            .pointer("/provider/settings/executionRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .unwrap_or_else(|| ResourceRef::parse(HOST_REF).expect("Host ref"));
        let runtime = self.runtime()?;
        let vm_id = VmId::new(holder.name().as_str());
        let migration_intent = BundleOpId::new(format!("legacy-swtpm:vm:{}", vm_id.as_str()));
        let decision = runtime
            .tpm_device_is_admitted(
                &request.uid,
                &crate::shared_provider_driver::key_ref(&request.target),
                vm_id.as_str(),
                &request.operation_id,
                None,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let lifecycle = runtime
            .admit_internal_guest_lifecycle(holder.clone(), &request.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let authorization = crate::provider_effects::LifecycleAuthorization::from_lease(
            lifecycle.lease,
            holder,
            lifecycle.guest_uid,
            lifecycle.guest_generation,
            lifecycle.provider_assignment_generation,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let log_level = request
            .spec
            .pointer("/provider/settings/logLevel")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(20);
        let mut controllers = request
            .state
            .tpm_controllers
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controller = controllers
            .remove(&request.uid)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let result = crate::tpm_effect_port::finalize_device_tpm_controller(
            &self.state,
            &resolver,
            vm_id.clone(),
            migration_intent,
            decision,
            crate::tpm_effect_port::AdmittedTpmDevice::new(
                request.uid.clone(),
                crate::shared_provider_driver::key_ref(&request.target).clone(),
                self.zone.as_str(),
                execution_ref,
                authorization,
            ),
            tpm_state_intent(&request.uid, vm_id.as_str()),
            d2b_provider_device_tpm::SwtpmSettings { log_level },
            d2b_provider_device_tpm::SignedBinaryRef::from_core(
                d2b_provider_device_tpm::BinaryKind::Swtpm,
                tpm_opaque_bytes("d2b:tpm-binary/v1", vm_id.as_str()),
            ),
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
            &mut controller,
        );
        match result {
            Ok(_) => Ok(SharedProviderFinalize::Complete),
            Err(error) => {
                controllers.insert(request.uid.clone(), controller);
                tracing::debug!(
                    error = ?error,
                    device = %crate::shared_provider_driver::key_ref(&request.target).to_canonical_string(),
                    "TPM device controller finalize failed",
                );
                Err(SharedProviderEffectError::Unavailable)
            }
        }
    }

    async fn finalize_usbip_service(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let service_ref = crate::shared_provider_driver::key_ref(&request.target).to_canonical_string();
        let bindings = runtime
            .committed_resources_of_type(d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if bindings.iter().any(|binding| {
            binding.pointer("/spec/serviceRef").and_then(Value::as_str)
                == Some(service_ref.as_str())
        }) {
            return Ok(SharedProviderFinalize::Pending);
        }
        let (zone_uid, opted_in, mut port) = self.usbip_service_port(&runtime, request).await?;
        if !opted_in {
            return Ok(SharedProviderFinalize::Complete);
        }
        let mut lifecycle = d2b_provider_device_usbip::ServiceLifecycle::new(
            zone_uid.clone(),
            request.uid.clone(),
        );
        lifecycle
            .activate(true, zone_uid, &mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut supervisor = d2b_provider_device_usbip::UsbipSupervisor::new(lifecycle);
        supervisor
            .finalize(&mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        self.usbip_services
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?
            .remove(&request.uid);
        Ok(SharedProviderFinalize::Complete)
    }

    async fn finalize_gpu(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let mut controllers = request
            .state
            .gpu_controllers
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let admission = controllers
            .get(&request.uid)
            .and_then(|controller| controller.admission().cloned())
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let resolver = crate::load_bundle_resolver(&self.state)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let runtime = self.runtime()?;
        let opened_devices = take_gpu_opened_devices(request.state, &request.uid)?;
        let mut controller = controllers
            .remove(&request.uid)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let mut port = DaemonGpuLifecyclePort {
            state: Arc::clone(&self.state),
            runtime,
            resolver,
            device_ref: crate::shared_provider_driver::key_ref(&request.target).clone(),
            device_uid: request.uid.clone(),
            holder_ref: admission.owner().holder_ref().clone(),
            generation: admission.owner().generation(),
            settings: controller.settings().clone(),
            operation_id: request.operation_id.clone(),
            state_maps: request.state,
            opened_devices,
        };
        let result = controller.finalize_lifecycle(&mut port).map_err(|error| {
            tracing::debug!(
                error = ?error,
                device = %crate::shared_provider_driver::key_ref(&request.target).to_canonical_string(),
                "GPU lifecycle finalize failed",
            );
            SharedProviderEffectError::Unavailable
        });
        match result {
            Ok(()) => Ok(SharedProviderFinalize::Complete),
            Err(error) => {
                let opened_devices = std::mem::take(&mut port.opened_devices);
                retain_gpu_opened_devices(request.state, &request.uid, opened_devices)?;
                controllers.insert(request.uid.clone(), controller);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use d2b_resource_runtime::error::{DriverFailure, DriverOp};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;

    use super::view_phase;

    /// One manager view carrying the given status classification, published
    /// generation, and durable deleting mark.
    fn phase_view(
        status: Option<ResourceStatus>,
        status_generation: Option<u64>,
        deleting: bool,
    ) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", "Volume", "phase-view"),
            uid: [0x42; 16],
            generation: 2,
            deleting,
            provenance: ResourceProvenance::Api,
            spec: b"{}".to_vec(),
            metadata: b"{}".to_vec(),
            owner_key: None,
            status,
            status_generation,
            status_projection: None,
        }
    }

    /// Issue #515: the phase gate (`live_phase` and `resource_value`, which
    /// both read `view_phase`) delegates to the canonical wire producer.
    /// Across the closed status vocabulary - and across the runtime flags
    /// (the durable deleting mark, an ungenerationed status, a stale
    /// generation) - the gate's phase equals the phase
    /// `ResourceView::wire_status` serves, so a future divergence fails here.
    #[test]
    fn view_phase_delegates_to_the_canonical_wire_phase() {
        let statuses = [
            None,
            Some(ResourceStatus::Pending),
            Some(ResourceStatus::Recovering),
            Some(ResourceStatus::Reconciling),
            Some(ResourceStatus::Ready),
            Some(ResourceStatus::Failed(DriverFailure::retryable(DriverOp::Reconcile))),
            Some(ResourceStatus::Failed(DriverFailure::terminal(DriverOp::Delete))),
            Some(ResourceStatus::Deleting),
        ];
        for deleting in [false, true] {
            for status in &statuses {
                for status_generation in [None, Some(1), Some(2), Some(3)] {
                    let view = phase_view(status.clone(), status_generation, deleting);
                    let canonical = view.wire_status()["phase"]
                        .as_str()
                        .expect("the canonical status always carries a phase")
                        .to_owned();
                    assert_eq!(
                        view_phase(&view),
                        canonical,
                        "phase divergence: status={status:?} status_generation={status_generation:?} deleting={deleting}",
                    );
                }
            }
        }
    }
}

