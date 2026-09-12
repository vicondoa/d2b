//! Bridge from the old-plane controller readers onto the new plane's
//! manager view (G5, KTD3/KTD4).
//!
//! Since KTD4 the bundle's controller-class `Process` rows (and every other
//! converted type) are served by the per-zone manager, while the controller
//! session machinery and the Core `Provider` handler still read the redb
//! store. Two readers are blinded by that split:
//!
//! - the controller-session path (`controller_context_is_current`,
//!   `persist_controller_session_evidence`, `fence_process_resources`), which
//!   must see a manager-served controller row as current, and
//! - the Core `Provider` handler's dependency observation, which must see the
//!   controller `Process` rows it owns (and its provider `Volume` rows)
//!   before it can report dependencies ready.
//!
//! This module owns the read-only seam over the manager's existing view
//! surface ([`ResourceManagerClient::get`] / [`ResourceManagerClient::list`]):
//! [`ControllerPlaneView`] answers one `Process` row read for the
//! controller-session path; a manager RPC failure is never reported as
//! absence, and a row the manager does not hold (`Ok(None)`) keeps the
//! caller on the durable store path.
//!
//! Nothing here writes *status*: converted rows keep the manager's
//! single-writer discipline (the row's actor owns its status, R11), and rows
//! the manager does not serve keep the durable store. The one write seam is
//! [`PlaneChildMutations`]: a provider controller's child commits for
//! converted types are owner-scoped manager ensures/removes, exactly the
//! shape the plane's Nix ingest uses, because a converted child written to
//! the pre-v3 store has no actor and therefore no launch path.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::ResourceKey as ManagerKey;
use d2b_resource_runtime::manager::{
    DesiredResource, MutationSubject, ResourceManagerClient, ResourceSelector, ResourceView,
};
use d2b_resource_runtime::spec_store::ResourceProvenance;
use d2b_resource_store::StoredResource;
use serde_json::Value;

use crate::resource_plane_v3::{PlaneRoute, route_resource_type};

// ---------------------------------------------------------------------------
// Controller-session path seam
// ---------------------------------------------------------------------------

/// The manager view the controller-session path reads one `Process` row
/// from. `Ok(None)` means the manager does not serve the row (an unconverted
/// or legacy row: the durable store owns it); an RPC failure is an error and
/// never absence.
#[async_trait]
pub(crate) trait ControllerPlaneView: Send + Sync + 'static {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError>;

    /// The manager rows of one resource type (U12 reader bridge): the
    /// store-shaped readers merge these for converted types. The default is
    /// empty so a fixture that only serves the controller-session process
    /// row keeps working; an RPC failure is never reported as absence.
    async fn rows_of_type(
        &self,
        _resource_type: &str,
    ) -> Result<Vec<ResourceView>, ResourceError> {
        Ok(Vec::new())
    }
}

/// Production seam over one zone's manager client (the plane's published
/// client; cloned cheaply per read).
#[derive(Clone)]
pub(crate) struct ManagerControllerPlaneView {
    client: ResourceManagerClient,
    zone: ZoneId,
}

impl ManagerControllerPlaneView {
    pub(crate) fn new(client: ResourceManagerClient, zone: ZoneId) -> Self {
        Self { client, zone }
    }
}

#[async_trait]
impl ControllerPlaneView for ManagerControllerPlaneView {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError> {
        if process_ref.resource_type().as_str() != "Process" {
            return Ok(None);
        }
        self.client
            .get(ManagerKey::new(
                self.zone.as_str(),
                process_ref.resource_type().as_str(),
                process_ref.name().as_str(),
            ))
            .await
    }

    async fn rows_of_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<ResourceView>, ResourceError> {
        self.client
            .list(ResourceSelector {
                zone: Some(self.zone.as_str().to_owned()),
                type_name: Some(resource_type.to_owned()),
                owner: None,
            })
            .await
    }
}

/// Production seam over the composition's published per-zone plane table.
///
/// The composition hands this table to a runtime at the top of its per-zone
/// loop and fills it only after the loop, so a view resolved at attach time
/// is permanently empty. This seam resolves the zone's plane per read, so the
/// controller-session path sees manager-served rows no matter when the
/// composition publishes them.
pub(crate) struct PublishedPlaneControllerView {
    planes: Arc<parking_lot::Mutex<HashMap<String, Arc<crate::resource_plane_v3::ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl PublishedPlaneControllerView {
    pub(crate) fn new(
        planes: Arc<parking_lot::Mutex<HashMap<String, Arc<crate::resource_plane_v3::ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }
}

#[async_trait]
impl ControllerPlaneView for PublishedPlaneControllerView {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError> {
        let Some(plane) = self.planes.lock().get(self.zone.as_str()).cloned() else {
            return Ok(None);
        };
        ManagerControllerPlaneView::new(plane.client().clone(), self.zone.clone())
            .process_view(process_ref)
            .await
    }

    async fn rows_of_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<ResourceView>, ResourceError> {
        let Some(plane) = self.planes.lock().get(self.zone.as_str()).cloned() else {
            return Ok(Vec::new());
        };
        ManagerControllerPlaneView::new(plane.client().clone(), self.zone.clone())
            .rows_of_type(resource_type)
            .await
    }
}

// ---------------------------------------------------------------------------
// Live controller-session evidence
// ---------------------------------------------------------------------------

/// The live admitted controller session for one controller `Process` row.
///
/// The manager-served row carries no durable status (R11/AE6), so the
/// evidence the Core `Provider` handler reads as
/// `status.resource.controllerSession` is provided from the authoritative
/// live state: the admitted session and its live service task. Every
/// uncertainty - not admitted, a different row identity or generation, a
/// finished task - answers `None`, so no caller can synthesize `ready: true`.
pub(crate) trait LiveControllerSessionEvidence: Send + Sync + 'static {
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &ResourceUid,
        generation: ResourceGeneration,
    ) -> Option<Value>;
}

/// Map a manager row's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (same mapping the converted drivers use).
pub(crate) fn row_uid(bytes: &[u8; 16]) -> Option<ResourceUid> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).ok()
}

// ---------------------------------------------------------------------------
// Controller child mutations onto the manager plane
// ---------------------------------------------------------------------------

/// Which plane commits one provider-controller child mutation (KTD4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildMutationRoute {
    /// The manager owns the type: the mutation is an owner-scoped
    /// ensure/remove on the new plane.
    Manager,
    /// The pre-v3 store owns the type: the mutation keeps the registered
    /// controller API path.
    Legacy,
}

/// Classify one child type onto its committing plane, exactly as
/// [`route_resource_type`] classifies every other surface: a converted child
/// is never written to the pre-v3 store (no actor would ever realize it) and
/// an unconverted one never appears in the manager.
pub(crate) fn child_type_route(type_name: &str) -> ChildMutationRoute {
    match route_resource_type(type_name) {
        PlaneRoute::NewPlane => ChildMutationRoute::Manager,
        PlaneRoute::OldPlane => ChildMutationRoute::Legacy,
    }
}

/// Classify one child mutation by its target.
pub(crate) fn child_mutation_route(target: &ResourceRef) -> ChildMutationRoute {
    child_type_route(target.resource_type().as_str())
}

/// Child mutation failure at the manager boundary, projected onto the
/// provider session's closed error shape by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildMutationFailure {
    /// The requested identity, revision, or create-absent fence did not hold.
    Conflict,
    /// The row the mutation names is absent from the manager.
    NotFound,
    /// The payload is not a decodable canonical resource envelope.
    Invalid,
    /// The manager or its store could not answer; retryable.
    Unavailable,
}

/// Owner-scoped child mutations on the new plane for one provider session.
///
/// A provider controller's child commit for a converted type is not a store
/// mutation at all: the child lives in the manager, so the mutation is the
/// same owner-scoped `ensure`/`remove` the plane's Nix ingest applies
/// (`ResourceManagerClient::ensure` / `remove`). The port carries the row
/// payload the provider authored - the spec layer every driver decoder reads
/// plus the authored metadata envelope - and presents the owning resource as
/// the authored `metadata.ownerRef` under the owner-subject admission, not as
/// a linked `owner_uid`: the manager links ownership by uid, a Guest owner is
/// not a manager row (`Guest` stays on the pre-v3 plane), and the top-level
/// shape with the authored reference is exactly the shape
/// [`crate::resource_plane_v3::ResourcePlaneV3::ingest_nix_bundle`] gives
/// rows whose owner never appears in the plane. Every read of the row (public
/// API, `ctx.owner_key` fallback, session relist) sees the authored owner
/// reference.
pub(crate) struct PlaneChildMutations {
    plane: Arc<crate::resource_plane_v3::ResourcePlaneV3>,
    zone: ZoneId,
    owner_ref: ResourceRef,
}

impl PlaneChildMutations {
    pub(crate) fn new(
        plane: Arc<crate::resource_plane_v3::ResourcePlaneV3>,
        zone: ZoneId,
        owner_ref: ResourceRef,
    ) -> Self {
        Self {
            plane,
            zone,
            owner_ref,
        }
    }

    /// The owning resource reference every mutation of this session is filed
    /// under.
    pub(crate) fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    fn key(&self, target: &ResourceRef) -> ManagerKey {
        ManagerKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    fn subject(&self) -> MutationSubject {
        d2b_resource_api::manager_backend::resource_owner_subject(&ManagerKey::new(
            self.zone.as_str(),
            self.owner_ref.resource_type().as_str(),
            self.owner_ref.name().as_str(),
        ))
    }

    /// The committed row through the canonical manager-row rendering, or
    /// `None` when the manager does not hold it. A manager RPC failure is an
    /// error, never absence (G5).
    pub(crate) async fn current(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<StoredResource>, ChildMutationFailure> {
        let view = self
            .plane
            .client()
            .get(self.key(target))
            .await
            .map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    target = %target.to_canonical_string(),
                    error = %error,
                    "child mutation bridge: manager row read failed",
                );
                ChildMutationFailure::Unavailable
            })?;
        view.map(|view| {
            d2b_resource_api::manager_backend::manager_row_stored(&view)
                .map_err(|_| ChildMutationFailure::Invalid)
        })
        .transpose()
    }

    /// Every manager row of the requested converted types (the relist and
    /// finalization read). Unconverted types contribute nothing: their rows
    /// stay on the durable store path.
    pub(crate) async fn rows_of_types(
        &self,
        resource_types: &[&str],
    ) -> Result<Vec<StoredResource>, ChildMutationFailure> {
        let mut rows = Vec::new();
        for resource_type in resource_types {
            if child_type_route(resource_type) != ChildMutationRoute::Manager {
                continue;
            }
            let views = self
                .plane
                .client()
                .list(ResourceSelector {
                    zone: Some(self.zone.as_str().to_owned()),
                    type_name: Some((*resource_type).to_owned()),
                    owner: None,
                })
                .await
                .map_err(|error| {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        resource_type,
                        error = %error,
                        "child mutation bridge: manager row list failed",
                    );
                    ChildMutationFailure::Unavailable
                })?;
            for view in views {
                rows.push(
                    d2b_resource_api::manager_backend::manager_row_stored(&view)
                        .map_err(|_| ChildMutationFailure::Invalid)?,
                );
            }
        }
        Ok(rows)
    }

    /// Commit one provider-authored child envelope under the create-absent
    /// fence (the store mutation's `ExpectedRevision::CreateAbsent`): an
    /// existing row is a conflict, never an overwrite.
    pub(crate) async fn ensure(
        &self,
        target: &ResourceRef,
        canonical_envelope: &[u8],
    ) -> Result<StoredResource, ChildMutationFailure> {
        if self.current(target).await?.is_some() {
            return Err(ChildMutationFailure::Conflict);
        }
        self.apply(target, canonical_envelope).await
    }

    /// Replace one child's spec under the caller's exact uid/revision fence
    /// (the store mutation's `ExpectedRevision::Exact` plus `expected_uid`).
    pub(crate) async fn update(
        &self,
        target: &ResourceRef,
        expected_uid: &ResourceUid,
        expected_revision: ZoneRevision,
        canonical_envelope: &[u8],
    ) -> Result<StoredResource, ChildMutationFailure> {
        let current = self
            .current(target)
            .await?
            .ok_or(ChildMutationFailure::NotFound)?;
        if current.uid != *expected_uid || current.revision != expected_revision {
            return Err(ChildMutationFailure::Conflict);
        }
        self.apply(target, canonical_envelope).await
    }

    /// Durable deletion: the manager marks the row deleting and cascades; the
    /// child's own actor owns the cleanup (R10, F3). Idempotent.
    pub(crate) async fn remove(&self, target: &ResourceRef) -> Result<(), ChildMutationFailure> {
        self.plane
            .client()
            .remove(self.subject(), self.key(target))
            .await
            .map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    target = %target.to_canonical_string(),
                    error = %error,
                    "child mutation bridge: manager remove failed",
                );
                ChildMutationFailure::Unavailable
            })?;
        if matches!(target.resource_type().as_str(), "Volume" | "VolumeBinding") {
            self.refresh_registry().await;
        }
        Ok(())
    }

    async fn apply(
        &self,
        target: &ResourceRef,
        canonical_envelope: &[u8],
    ) -> Result<StoredResource, ChildMutationFailure> {
        let desired = desired_child_resource(&self.zone, target, canonical_envelope)?;
        self.plane
            .client()
            .ensure(self.subject(), None, desired)
            .await
            .map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    target = %target.to_canonical_string(),
                    error = %error,
                    "child mutation bridge: manager ensure failed",
                );
                match error {
                    ResourceError::DeletingConflict { .. } => ChildMutationFailure::Conflict,
                    _ => ChildMutationFailure::Unavailable,
                }
            })?;
        if matches!(target.resource_type().as_str(), "Volume" | "VolumeBinding") {
            self.refresh_registry().await;
        }
        self.current(target)
            .await?
            .ok_or(ChildMutationFailure::Unavailable)
    }

    /// Re-register the durable rows the production effects resolve
    /// per-resource anchors from: the manager is the only writer, so a Volume
    /// or VolumeBinding committed after the plane's durable load is only
    /// observable through a reload, and an unregistered Volume root resolves
    /// as a `volume-anchor` failure forever.
    async fn refresh_registry(&self) {
        if let Err(error) = self.plane.reload_registry().await {
            tracing::warn!(
                zone = %self.zone.as_str(),
                error = %error,
                "child mutation bridge: per-resource anchor reload failed; retrying on the next commit",
            );
        }
    }
}

/// The authored owner reference of one provider child envelope: the fence
/// every commit path checks against the batch's fenced owner. The provider
/// payload is not a complete store envelope (the row identity is the store's
/// to stamp), so this reads the authored JSON rather than a strict envelope.
pub(crate) fn child_envelope_owner(canonical_envelope: &[u8]) -> Option<ResourceRef> {
    let value: Value = serde_json::from_slice(canonical_envelope).ok()?;
    value
        .get("metadata")?
        .get("ownerRef")?
        .as_str()
        .and_then(|owner| ResourceRef::parse(owner).ok())
}

/// Split one provider-authored canonical child envelope into the manager's
/// desired resource (KTD2): the spec layer bytes the per-type driver decoder
/// reads, and the authored metadata envelope (owner reference, finalizers,
/// management fields). The authored identity is still the caller's fence -
/// the envelope must name the target and Zone it claims - but the row's uid,
/// generation, and revision stay the manager's to stamp, exactly as the store
/// mutation path stamps them.
pub(crate) fn desired_child_resource(
    zone: &ZoneId,
    target: &ResourceRef,
    canonical_envelope: &[u8],
) -> Result<DesiredResource, ChildMutationFailure> {
    let value: Value =
        serde_json::from_slice(canonical_envelope).map_err(|_| ChildMutationFailure::Invalid)?;
    let metadata = value
        .get("metadata")
        .filter(|metadata| metadata.is_object())
        .ok_or(ChildMutationFailure::Invalid)?;
    if metadata.get("name").and_then(Value::as_str) != Some(target.name().as_str())
        || metadata.get("zone").and_then(Value::as_str) != Some(zone.as_str())
    {
        return Err(ChildMutationFailure::Invalid);
    }
    let canonical = |value: &Value| -> Result<Vec<u8>, ChildMutationFailure> {
        let bytes = serde_json::to_vec(value).map_err(|_| ChildMutationFailure::Invalid)?;
        Ok(CanonicalJsonValue::parse(&bytes)
            .map_err(|_| ChildMutationFailure::Invalid)?
            .to_canonical_bytes())
    };
    let spec = canonical(value.get("spec").ok_or(ChildMutationFailure::Invalid)?)?;
    let metadata = canonical(metadata)?;
    Ok(DesiredResource {
        key: ManagerKey::new(
            zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        ),
        spec,
        metadata,
        provenance: ResourceProvenance::Resource,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_resource_runtime::identity::ResourceProvenance;
    use serde_json::json;

    /// The Guest family's four deterministic child roles are converted types:
    /// their commits must land on the manager (the pre-v3 store has no actor
    /// to launch them), while a type that is still unconverted keeps the
    /// durable registered-controller path - the same partition every other
    /// surface applies.
    #[test]
    fn child_mutations_route_converted_types_to_the_manager() {
        for converted in ["Guest", "Process", "Endpoint", "Volume", "VolumeBinding"] {
            assert_eq!(
                child_type_route(converted),
                ChildMutationRoute::Manager,
                "{converted} is a converted type"
            );
        }
        for legacy in ["EphemeralProcess"] {
            assert_eq!(
                child_type_route(legacy),
                ChildMutationRoute::Legacy,
                "{legacy} stays on the pre-v3 plane"
            );
        }
        assert_eq!(
            child_mutation_route(
                &ResourceRef::parse("Process/acceptance-guest-vmm").expect("child ref")
            ),
            ChildMutationRoute::Manager,
        );
    }

    /// Regression (vmCheck guest preflight): every row of the Cloud
    /// Hypervisor controller's deterministic guest child set - the VMM
    /// Process, both control endpoints, the system Volume - and the Guest's
    /// worker VolumeBinding route to the manager, where the per-type drivers
    /// realize them. Routing any of them to the pre-v3 store leaves a row
    /// with no actor and the guest's readiness gate never converges.
    #[test]
    fn cloud_hypervisor_guest_children_reach_their_drivers_through_the_manager() {
        use d2b_provider_runtime_cloud_hypervisor::ChildRole;

        let guest = ResourceRef::parse("Guest/acceptance-guest").expect("guest ref");
        for role in [
            ChildRole::VmmProcess,
            ChildRole::ChApiEndpoint,
            ChildRole::GuestControlEndpoint,
            ChildRole::SystemVolume,
        ] {
            let child =
                d2b_provider_runtime_cloud_hypervisor::deterministic_child_ref(&guest, role)
                    .expect("deterministic Cloud Hypervisor child");
            assert_eq!(
                child_mutation_route(&child),
                ChildMutationRoute::Manager,
                "the Cloud Hypervisor child {child} must commit through the manager",
            );
            assert_eq!(
                crate::resource_plane_v3::route_resource_type(child.resource_type().as_str()),
                crate::resource_plane_v3::PlaneRoute::NewPlane,
                "the child's type must be served by a converted driver",
            );
        }
        let worker_binding =
            ResourceRef::parse("VolumeBinding/acceptance-guest-work").expect("binding ref");
        assert_eq!(
            child_mutation_route(&worker_binding),
            ChildMutationRoute::Manager,
            "the guest's worker VolumeBinding must commit through the manager",
        );
    }

    /// One provider-authored child envelope as the Cloud Hypervisor controller
    /// renders it (`materialize_child_payload`): a spec layer and authored
    /// metadata, with the row identity left to the store.
    fn child_envelope(name: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Process",
            "metadata": {
                "name": name,
                "zone": "work",
                "ownerRef": "Guest/acceptance-guest",
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
            },
            "spec": {
                "providerRef": "Provider/system-minijail",
                "executionRef": "Host/host-system",
                "processClass": "worker",
                "template": "cloud-hypervisor-runner",
                "desiredLifecycle": "stopped",
                "sandbox": {},
            },
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "resource": {},
            },
        }))
        .expect("child envelope")
    }

    /// The payload split: the manager row keeps the spec layer the Process
    /// driver decodes (never the envelope, which would decode as an empty
    /// base) and the authored metadata that renders the owner reference into
    /// every read.
    #[test]
    fn desired_child_resource_splits_the_authored_envelope() {
        let zone = ZoneId::parse("work").expect("zone");
        let target = ResourceRef::parse("Process/acceptance-guest-vmm").expect("target");
        let desired = desired_child_resource(&zone, &target, &child_envelope("acceptance-guest-vmm"))
            .expect("desired child");

        assert_eq!(
            desired.key,
            ManagerKey::new("work", "Process", "acceptance-guest-vmm")
        );
        assert_eq!(desired.provenance, ResourceProvenance::Resource);
        let spec: Value = serde_json::from_slice(&desired.spec).expect("spec layer");
        assert_eq!(spec.get("processClass").and_then(Value::as_str), Some("worker"));
        assert!(
            spec.get("metadata").is_none(),
            "the driver decodes the spec layer, not the envelope: {spec}"
        );
        let metadata: Value = serde_json::from_slice(&desired.metadata).expect("metadata");
        assert_eq!(
            metadata.get("ownerRef").and_then(Value::as_str),
            Some("Guest/acceptance-guest")
        );
        assert_eq!(
            child_envelope_owner(&child_envelope("acceptance-guest-vmm")),
            Some(ResourceRef::parse("Guest/acceptance-guest").expect("owner")),
        );
    }

    /// The authored identity stays the caller's fence: an envelope that names
    /// another row is refused instead of committing the wrong key.
    #[test]
    fn desired_child_resource_refuses_an_envelope_that_names_another_row() {
        let zone = ZoneId::parse("work").expect("zone");
        let target = ResourceRef::parse("Process/acceptance-guest-vmm").expect("target");
        assert_eq!(
            desired_child_resource(&zone, &target, &child_envelope("other-vmm")),
            Err(ChildMutationFailure::Invalid),
        );
        assert_eq!(
            desired_child_resource(
                &ZoneId::parse("other").expect("zone"),
                &target,
                &child_envelope("acceptance-guest-vmm"),
            ),
            Err(ChildMutationFailure::Invalid),
        );
    }

    /// One Cloud Hypervisor controller Endpoint child envelope as
    /// `materialize_child_payload` renders it for a Guest's control endpoint:
    /// the authored create body plus the provider's materialized defaults.
    fn control_endpoint_envelope() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Endpoint",
            "metadata": {
                "name": "acceptance-guest-guest-control",
                "zone": "work",
                "ownerRef": "Guest/acceptance-guest",
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
            },
            "spec": {
                "providerRef": "Provider/runtime-cloud-hypervisor",
                "producerRef": "Guest/acceptance-guest",
                "purpose": "guest-control",
                "endpointClass": "control",
                "transport": "opaque-carriage",
                "locality": "cross-domain",
                "visibility": "provider",
                "attachmentPolicy": { "supported": true, "maxAttachments": 1 },
                "consumerPolicy": { "allowedOperations": ["resolve", "attach", "observe"] },
                "lifecyclePolicy": "recycle-with-producer",
            },
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "resource": {},
            },
        }))
        .expect("endpoint envelope")
    }

    /// The conversion keeps every authored Endpoint field: the committed
    /// row's spec layer must decode as the closed Endpoint contract and be
    /// admitted by the Endpoint driver - a dropped field (the all-null
    /// projection the fixture summary prints shows none of them) would fail
    /// the row at validate with `endpoint-spec-invalid` /
    /// `endpoint-shape-unsupported`.
    #[test]
    fn desired_child_resource_keeps_the_authored_endpoint_spec() {
        let zone = ZoneId::parse("work").expect("zone");
        let target =
            ResourceRef::parse("Endpoint/acceptance-guest-guest-control").expect("target");
        let desired = desired_child_resource(&zone, &target, &control_endpoint_envelope())
            .expect("desired child");
        assert_eq!(
            desired.key,
            ManagerKey::new("work", "Endpoint", "acceptance-guest-guest-control")
        );

        let envelope: d2b_contracts_resource::v3::ResourceSpec =
            serde_json::from_slice(&desired.spec).expect("spec");
        let spec: d2b_contracts_resource::v3::endpoint::EndpointSpec =
            serde_json::from_slice(&envelope.base_with_provider_ref().to_canonical_bytes())
                .expect("endpoint contract");
        assert_eq!(spec.purpose().as_str(), "guest-control");
        assert_eq!(
            spec.producer_ref(),
            &ResourceRef::parse("Guest/acceptance-guest").expect("producer")
        );
        assert_eq!(
            spec.provider_ref(),
            &ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("provider")
        );
        assert_eq!(
            crate::endpoint_driver::endpoint_realization(&spec),
            Some(crate::endpoint_driver::EndpointRealization::GuestControl),
            "the committed row must be the shape the Endpoint driver realizes"
        );
    }

    /// One Cloud Hypervisor controller Endpoint child envelope for the
    /// `ch-api` role, exactly as `materialize_child_payload` renders it: the
    /// create body binds the role to the guest's VMM Process
    /// (`GuestChildBatch::from_descriptor`), so the materialized locality is
    /// `host-local` (`cross-domain` is materialized exactly for a
    /// `Guest/`-produced endpoint).
    fn ch_api_endpoint_envelope() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Endpoint",
            "metadata": {
                "name": "acceptance-guest-ch-api",
                "zone": "work",
                "ownerRef": "Guest/acceptance-guest",
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
            },
            "spec": {
                "providerRef": "Provider/runtime-cloud-hypervisor",
                "producerRef": "Process/acceptance-guest-vmm",
                "purpose": "ch-api",
                "endpointClass": "control",
                "transport": "opaque-carriage",
                "locality": "host-local",
                "visibility": "provider",
                "attachmentPolicy": { "supported": true, "maxAttachments": 1 },
                "consumerPolicy": { "allowedOperations": ["resolve", "attach", "observe"] },
                "lifecyclePolicy": "recycle-with-producer",
            },
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "resource": {},
            },
        }))
        .expect("endpoint envelope")
    }

    /// Regression (vmCheck guest preflight): the committed `ch-api` row is
    /// the provider's own shape - the VMM Process producer, materialized
    /// host-local - and the Endpoint driver must admit it. The admission set
    /// previously demanded the Guest-produced cross-domain shape for the
    /// whole control family, so this row failed `validate` terminally
    /// (`endpoint-shape-unsupported`), the guest's endpoint-publication stage
    /// refused its `Failed` phase, and the Guest never reached Ready.
    #[test]
    fn desired_child_resource_keeps_the_authored_ch_api_spec() {
        let zone = ZoneId::parse("work").expect("zone");
        let target = ResourceRef::parse("Endpoint/acceptance-guest-ch-api").expect("target");
        let desired = desired_child_resource(&zone, &target, &ch_api_endpoint_envelope())
            .expect("desired child");
        assert_eq!(
            desired.key,
            ManagerKey::new("work", "Endpoint", "acceptance-guest-ch-api")
        );

        let envelope: d2b_contracts_resource::v3::ResourceSpec =
            serde_json::from_slice(&desired.spec).expect("spec");
        let spec: d2b_contracts_resource::v3::endpoint::EndpointSpec =
            serde_json::from_slice(&envelope.base_with_provider_ref().to_canonical_bytes())
                .expect("endpoint contract");
        assert_eq!(spec.purpose().as_str(), "ch-api");
        assert_eq!(
            spec.producer_ref(),
            &ResourceRef::parse("Process/acceptance-guest-vmm").expect("producer")
        );
        assert_eq!(
            spec.locality(),
            d2b_contracts_resource::v3::endpoint::EndpointLocality::HostLocal
        );
        assert_eq!(
            crate::endpoint_driver::endpoint_realization(&spec),
            Some(crate::endpoint_driver::EndpointRealization::GuestControl),
            "the committed ch-api row must be the shape the Endpoint driver realizes"
        );
    }
}
