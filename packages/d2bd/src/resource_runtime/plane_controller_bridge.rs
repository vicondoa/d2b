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
//! surface ([`ResourceManagerClient::get`] / [`ResourceManagerClient::list`])
//! plus the two adapters the daemon wires:
//!
//! - [`ControllerPlaneView`] answers one `Process` row read for the
//!   controller-session path; a manager RPC failure is never reported as
//!   absence, and a row the manager does not hold (`Ok(None)`) keeps the
//!   caller on the durable store path.
//! - [`PlaneAwareControllerApi`] decorates the registered controller API:
//!   for a `Provider` target it appends one synthesized
//!   [`DependencySnapshot`] per manager-served `Process`/`Volume` row the
//!   Provider owns. Readiness comes from
//!   [`ResourceView::observed_status`] (generation-filtered: a stale `Ready`
//!   never passes), and a controller `Process` row's session evidence is the
//!   live admitted session ([`LiveControllerSessionEvidence`]) - the
//!   manager has no durable status channel (R11/AE6), so the live session
//!   *is* the evidence and every unknown fails closed (`ready: false`).
//!
//! Nothing here writes: converted rows keep the manager's single-writer
//! discipline, and rows the manager does not serve keep the durable store.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::ResourceKey as ManagerKey;
use d2b_resource_runtime::manager::{ResourceManagerClient, ResourceSelector, ResourceView};
use serde_json::Value;

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
/// is permanently empty. This seam resolves the zone's plane per read - the
/// same lazy lookup [`ManagerPlaneDependencyRows`] uses - so the
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
