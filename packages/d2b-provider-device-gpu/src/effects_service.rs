//! The provider-owned implementation of the GPU family's lifecycle port
//! (U12 gpu step): the family serves its effects over the daemon-supplied
//! facets instead of a daemon-built port.
//!
//! The port is the [`GpuLifecycleEffectPort`] the Device controller calls.
//! The declared Device-owned worker rows are resolved through the manager
//! child surface and gated on the phase their Process controller publishes;
//! the Host-global authority leases are admitted and released through the
//! daemon-supplied runtime facet ([`crate::facets::GpuRuntime`]); and the
//! per-resource lease cache the port keeps is supplied by the driver's own
//! state, never daemon state.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_core_controller::authority::{AuthorityLease, AuthorityRequest};
use d2b_provider_toolkit::SharedProviderChildSurface;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::ResourceStatus;

use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::authority::{
    GpuClosureProof, GpuPlatformToken, GpuPrincipalToken, GpuProcessIdentity,
    GpuProcessObservation,
};
use crate::effects::{
    GpuEffectError, GpuEffectTokenSet, GpuLaunchTicket, GpuLifecycleEffectPort,
};
use crate::facets::GpuRuntime;
use crate::process::GpuProcessRole;

use crate::workers::{GpuWorkerSpec, VideoWorkerSpec};

/// Drive one async child-surface call on the runtime captured at
/// construction (the sync `GpuLifecycleEffectPort` boundary, U13
/// synchronous path): `block_in_place` is a no-op on dedicated daemon
/// threads and the correct escape from a multi-threaded runtime worker;
/// callers never run on a current-thread runtime.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn drive_sync<T>(handle: &tokio::runtime::Handle, future: impl Future<Output = T>) -> T {
    tokio::task::block_in_place(|| handle.block_on(future))
}

/// The canonical wire phase of one manager view (the daemon's shared
/// `view_phase` helper, moved with the port): the published status
/// classification, or `Pending` for a row without one.
fn view_phase(view: &ResourceView) -> &'static str {
    view.observed_status()
        .as_ref()
        .map(ResourceStatus::wire_phase)
        .unwrap_or("Pending")
}

/// The declared Device-owned GPU worker rows, resolved through the manager
/// child surface (U17, KTD13).
///
/// The worker rows are bundle-declared (`Process/gpu-<device>` with template
/// `gpu-worker`/`gpu-render-node`, `Process/video-<device>` with
/// `video-worker`), so the port reads them and gates on the phase their
/// Process controller publishes: the spec stays the bundle's (re-authoring it
/// here would differ byte-wise from the seeded row and mutate it on every
/// pass), and exactly one component - the Process controller - decides when a
/// worker lives, restarts, is adopted across daemon restarts, drains, and is
/// torn down. The declared row is also the worker's durable identity: the
/// opaque process token is derived from the row's durable uid and generation,
/// so it is observable after a daemon restart (the retired pid/start-time
/// digest was not).
pub struct DeclaredWorkerGpuPort<'a> {
    /// The daemon-supplied GPU runtime: the Host-global authority index one
    /// Device's leases are admitted to and released from.
    runtime: Arc<dyn GpuRuntime>,
    /// The per-resource authority lease cache the calling driver owns (old
    /// `gpu_authority_leases`), keyed by lease token.
    gpu_authority_leases: Arc<Mutex<BTreeMap<[u8; 16], AuthorityLease>>>,
    /// The daemon's runtime, captured at construction: the sync
    /// `GpuLifecycleEffectPort` boundary (the Provider crate's controller is
    /// synchronous) drives its async child-surface work on this handle (U13
    /// synchronous path, R11 inventory note).
    runtime_handle: tokio::runtime::Handle,
    /// Manager-routed child surface of the requiring Device.
    children: &'a dyn SharedProviderChildSurface,
    zone: String,
    device_ref: ResourceRef,
    device_uid: ResourceUid,
    holder_ref: ResourceRef,
    generation: ResourceGeneration,
    operation_id: String,
}

impl<'a> DeclaredWorkerGpuPort<'a> {
    /// The declared row prefix of one worker role
    /// (`Process/gpu-<device>` / `Process/video-<device>`).
    fn role_prefix(role: GpuProcessRole) -> &'static str {
        match role {
            GpuProcessRole::FullGpu
            | GpuProcessRole::RenderNode => "gpu",
            GpuProcessRole::Video => "video",
        }
    }

    /// The closed templates one role's declared row may carry. The video
    /// sidecar declares either the plain vaapi posture or the NVIDIA decode
    /// posture; which one the row carries is the owning Device's declared
    /// `videoNvidiaDecode` setting, so the declared row's own template - not
    /// a fixed constant - is the launch identity.
    const fn role_templates(
        role: GpuProcessRole,
    ) -> &'static [&'static str] {
        match role {
            GpuProcessRole::FullGpu => &["gpu-worker"],
            GpuProcessRole::RenderNode => &["gpu-render-node"],
            GpuProcessRole::Video => {
                &["video-worker", "video-worker-nvidia"]
            }
        }
    }

    /// The declared row's template, when it names one of the role's closed
    /// templates; anything else is a row this role cannot launch through.
    fn declared_row_template(
        view: &ResourceView,
        role: GpuProcessRole,
    ) -> Result<&'static str, GpuEffectError> {
        let spec: Value = serde_json::from_slice(&view.spec)
            .map_err(|_| GpuEffectError::SpawnRejected)?;
        let template = spec
            .get("template")
            .and_then(Value::as_str)
            .ok_or(GpuEffectError::SpawnRejected)?;
        Self::role_templates(role)
            .iter()
            .copied()
            .find(|candidate| *candidate == template)
            .ok_or(GpuEffectError::SpawnRejected)
    }

    /// The declared reference of one worker role
    /// (`Process/gpu-<device>` / `Process/video-<device>`).
    fn worker_ref(
        &self,
        role: GpuProcessRole,
    ) -> Result<ResourceRef, GpuEffectError> {
        let prefix = Self::role_prefix(role);
        ResourceRef::parse(&format!("Process/{prefix}-{}", self.device_ref.name().as_str()))
            .map_err(|_| GpuEffectError::SpawnRejected)
    }

    /// The manager key of one declared worker row.
    fn worker_key(
        &self,
        role: GpuProcessRole,
    ) -> Result<ResourceKey, GpuEffectError> {
        Ok(ResourceKey::new(
            self.zone.as_str(),
            "Process",
            self.worker_ref(role)?.name().as_str(),
        ))
    }

    /// The requiring Device's own key (the row owner fence).
    fn device_key(&self) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            "Device",
            self.device_ref.name().as_str(),
        )
    }

    /// The live view of one declared worker row. A row of another Device, or
    /// one whose declared template is not the role's, is refused rather than
    /// read as this worker's evidence.
    fn worker_view(
        &self,
        role: GpuProcessRole,
    ) -> Result<Option<ResourceView>, GpuEffectError> {
        let key = self.worker_key(role)?;
        // Sync `GpuLifecycleEffectPort` boundary (U13 synchronous path): the
        // Provider crate's controller is synchronous, so the async child
        // surface drives on the daemon runtime captured at construction.
        let view = drive_sync(&self.runtime_handle, self.children.view(&key))
            .map_err(|_| GpuEffectError::SpawnRejected)?;
        let Some(view) = view else {
            return Ok(None);
        };
        if view.owner_key.as_ref() != Some(&self.device_key()) {
            return Err(GpuEffectError::StaleDeviceIdentity);
        }
        Self::declared_row_template(&view, role)?;
        Ok(Some(view))
    }

    /// The opaque process token of one declared row: deterministic in the
    /// row's durable uid and generation, so the same identity is observed
    /// after a daemon restart and never restarts on a pid reuse.
    fn row_process_token(view: &ResourceView) -> [u8; 16] {
        let mut digest = Sha256::new();
        digest.update(b"d2b:gpu-process-row/v1");
        digest.update(view.uid);
        digest.update(view.generation.to_be_bytes());
        let digest: [u8; 32] = digest.finalize().into();
        digest[..16].try_into().expect("fixed process token length")
    }

    /// Mint (or re-derive) one worker identity from its declared row.
    fn row_identity(
        view: &ResourceView,
        role: GpuProcessRole,
        principal: &GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> GpuProcessIdentity {
        GpuProcessIdentity::from_core(
            Self::row_process_token(view),
            role,
            principal.clone(),
            platform.clone(),
            generation,
        )
    }

    /// Classify one declared row as observation evidence for `identity`.
    ///
    /// The row's deterministic token decides stale-versus-current, and the
    /// row's published phase decides whether a current token names a live
    /// worker at all: a row the Process controller reports `Pending` (a
    /// relaunch in flight or waiting out its restart backoff) or `Failed`
    /// publishes no live worker, so a matching token on it is `Missing` -
    /// never `Matching`. Reading the token alone let a restart during an
    /// outage adopt the dead worker's identity as live.
    fn row_observation(
        view: &ResourceView,
        identity: &GpuProcessIdentity,
    ) -> GpuProcessObservation {
        let observed = Self::row_identity(
            view,
            identity.role(),
            identity.principal(),
            identity.platform(),
            identity.generation(),
        );
        if &observed != identity {
            // The declared row was replaced under a new uid or generation:
            // the identity this actor holds is stale, not ambiguous.
            return GpuProcessObservation::StaleIdentity;
        }
        if view_phase(view) != "Ready" {
            return GpuProcessObservation::Missing;
        }
        GpuProcessObservation::Matching(observed)
    }

    /// Observe one declared worker row as the launch/observation evidence for
    /// one role: `Ready` is a live worker, `Pending` is retryable, and a
    /// failed row is a refusal.
    fn declared_worker(
        &self,
        role: GpuProcessRole,
    ) -> Result<ResourceView, GpuEffectError> {
        let view = self
            .worker_view(role)?
            .ok_or(GpuEffectError::SpawnRejected)?;
        match view_phase(&view) {
            "Ready" => Ok(view),
            "Failed" => Err(GpuEffectError::SpawnRejected),
            _ => Err(GpuEffectError::Transient),
        }
    }

    fn scope_digest(device_uid: &ResourceUid, operation_id: &str) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b:gpu-runtime-scope/v1");
        digest.update(device_uid.as_str().as_bytes());
        digest.update(operation_id.as_bytes());
        digest.finalize().into()
    }
}

impl GpuLifecycleEffectPort for DeclaredWorkerGpuPort<'_> {
    fn reserve_authority(
        &mut self,
        admission: &crate::authority::GpuAuthorityAdmission,
    ) -> Result<crate::authority::GpuAuthorityLease, GpuEffectError> {
        if admission.owner().device_uid() != &self.device_uid
            || admission.owner().holder_ref() != &self.holder_ref
            || admission.owner().generation() != self.generation
        {
            return Err(GpuEffectError::StaleDeviceIdentity);
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
        .map_err(|_| GpuEffectError::AuthorityConflict)?;
        let lease = self
            .runtime
            .admit_authority(request)
            .map_err(|_| GpuEffectError::AuthorityConflict)?;
        let token = lease.token_bytes();
        self.gpu_authority_leases
            .lock()
            .insert(token, lease);
        Ok(crate::authority::GpuAuthorityLease::from_core(token))
    }

    fn open_authorized_devices(
        &mut self,
        admission: &crate::authority::GpuAuthorityAdmission,
        tokens: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError> {
        if admission.owner().device_uid() != &self.device_uid
            || admission.owner().generation() != self.generation
            || !admission.owner().holder_ref().eq(&self.holder_ref)
        {
            return Err(GpuEffectError::StaleDeviceIdentity);
        }
        if tokens.is_empty() {
            return Err(GpuEffectError::StaleDeviceIdentity);
        }
        // The device grants travel in the launch intent's declared posture
        // (the closed `device_worker_posture` binds, plus the broker's own
        // render-node pre-open), so the ticket carries only the authority
        // scope the Device's generation is bound to; the launch itself is the
        // Process controller's.
        Ok(GpuLaunchTicket::from_core(
            Self::scope_digest(&self.device_uid, &self.operation_id)[..16]
                .try_into()
                .expect("fixed launch ticket length"),
        ))
    }

    fn start_gpu_worker(
        &mut self,
        spec: &GpuWorkerSpec,
        _ticket: &GpuLaunchTicket,
        principal: &GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        let role = spec.process().role();
        if !Self::role_templates(role).contains(&spec.template()) {
            return Err(GpuEffectError::SpawnRejected);
        }
        let view = self.declared_worker(role)?;
        // The declared row is the launch identity: the requested template
        // must be the one the row carries within the role's closed set.
        if Self::declared_row_template(&view, role)? != spec.template() {
            return Err(GpuEffectError::SpawnRejected);
        }
        Ok(Self::row_identity(
            &view, role, principal, platform, generation,
        ))
    }

    fn start_video_worker(
        &mut self,
        spec: &VideoWorkerSpec,
        _ticket: &GpuLaunchTicket,
        principal: &GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        if !Self::role_templates(GpuProcessRole::Video)
            .contains(&spec.template())
        {
            return Err(GpuEffectError::SpawnRejected);
        }
        let view = self.declared_worker(GpuProcessRole::Video)?;
        // The row declares one of the two closed video postures (plain or
        // NVIDIA decode, per the owning Device's setting): the request must
        // name the row's own template, never the other posture.
        if Self::declared_row_template(
            &view,
            GpuProcessRole::Video,
        )? != spec.template()
        {
            return Err(GpuEffectError::SpawnRejected);
        }
        Ok(Self::row_identity(
            &view,
            GpuProcessRole::Video,
            principal,
            platform,
            generation,
        ))
    }

    fn observe_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError> {
        let Some(view) = self.worker_view(identity.role())? else {
            return Ok(GpuProcessObservation::Missing);
        };
        Ok(Self::row_observation(&view, identity))
    }

    fn stop_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError> {
        if let Some(view) = self.worker_view(identity.role())? {
            let observed = Self::row_identity(
                &view,
                identity.role(),
                identity.principal(),
                identity.platform(),
                identity.generation(),
            );
            if &observed != identity {
                return Err(GpuEffectError::StaleDeviceIdentity);
            }
            let key = self.worker_key(identity.role())?;
            drive_sync(&self.runtime_handle, self.children.delete(&key))
                .map_err(|_| GpuEffectError::CloseUnconfirmed)?;
        }
        // The closure proof is the row's absence: the manager's delete runs
        // the Process controller's stop/finalize (drain, then teardown) before
        // the row is removed, so a still-present row is not proof.
        if self.worker_view(identity.role())?.is_some() {
            return Err(GpuEffectError::CloseUnconfirmed);
        }
        Ok(GpuClosureProof::from_core(
            identity.clone(),
        ))
    }

    fn release_authority(
        &mut self,
        lease: crate::authority::GpuAuthorityLease,
        _closures: &[GpuClosureProof],
    ) -> Result<(), GpuEffectError> {
        let token = *lease.as_bytes();
        let generic = self
            .gpu_authority_leases
            .lock()
            .remove(&token)
            .ok_or(GpuEffectError::AuthorityConflict)?;
        let result = self.runtime.release_authority(&generic);
        if result.is_err() {
            tracing::debug!(
                device = %self.device_ref.to_canonical_string(),
                "GPU authority lease release failed; lease restored",
            );
            self.gpu_authority_leases.lock().insert(token, generic);
            return Err(GpuEffectError::AuthorityConflict);
        }
        Ok(())
    }
}

/// The port's per-resource construction inputs, supplied by the calling
/// driver (the Device family's runtime) from the row and the driver state.
pub struct DeclaredWorkerGpuPortArgs<'a> {
    /// The daemon-supplied GPU runtime facet.
    pub runtime: Arc<dyn GpuRuntime>,
    /// The per-resource authority lease cache the driver owns.
    pub gpu_authority_leases: Arc<Mutex<BTreeMap<[u8; 16], AuthorityLease>>>,
    /// The runtime handle the sync port drives its async child surface on.
    pub runtime_handle: tokio::runtime::Handle,
    /// Manager-routed child surface of the requiring Device.
    pub children: &'a dyn SharedProviderChildSurface,
    /// The Zone the Device row lives in.
    pub zone: String,
    /// The Device row's reference.
    pub device_ref: ResourceRef,
    /// The Device row's uid.
    pub device_uid: ResourceUid,
    /// The Device row's owning holder reference.
    pub holder_ref: ResourceRef,
    /// The Device row's generation.
    pub generation: ResourceGeneration,
    /// The reconcile operation id the launch ticket scopes to.
    pub operation_id: String,
}

impl<'a> DeclaredWorkerGpuPort<'a> {
    /// Build the port from the calling driver's construction inputs.
    pub fn new(args: DeclaredWorkerGpuPortArgs<'a>) -> Self {
        Self {
            runtime: args.runtime,
            gpu_authority_leases: args.gpu_authority_leases,
            runtime_handle: args.runtime_handle,
            children: args.children,
            zone: args.zone,
            device_ref: args.device_ref,
            device_uid: args.device_uid,
            holder_ref: args.holder_ref,
            generation: args.generation,
            operation_id: args.operation_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_resource_runtime::error::{DriverFailure, DriverOp};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::resource::ResourceStatus;

    /// One declared worker row view: the given durable uid/generation and
    /// published status.
    fn worker_row_view(
        uid: [u8; 16],
        generation: u64,
        status: Option<ResourceStatus>,
    ) -> ResourceView {
        let status_generation = status.as_ref().map(|_| generation);
        ResourceView {
            key: ResourceKey::new("work", "Process", "gpu-corp-gpu"),
            uid,
            generation,
            deleting: false,
            provenance: ResourceProvenance::Resource,
            spec: b"{}".to_vec(),
            metadata: b"{}".to_vec(),
            owner_key: None,
            status,
            status_generation,
            status_projection: None,
        }
    }

    /// A restarting or refused declared worker row is never observation
    /// evidence. The row's token is deterministic in its uid and generation,
    /// so a `Pending` row (a relaunch in flight, or one waiting out its
    /// restart backoff) still matched the held identity and the GPU controller
    /// adopted the dead worker as live. The row's published phase gates the
    /// match; a row replaced under a new uid stays `StaleIdentity`.
    #[test]
    fn worker_observation_is_gated_on_the_declared_rows_phase() {
        let ready = worker_row_view([0x33; 16], 7, Some(ResourceStatus::Ready));
        let identity = GpuProcessIdentity::from_core(
            DeclaredWorkerGpuPort::row_process_token(&ready),
            GpuProcessRole::FullGpu,
            GpuPrincipalToken::from_core([0x11; 32]),
            GpuPlatformToken::from_core([0x22; 32]),
            ResourceGeneration::new(7).expect("generation"),
        );

        assert!(
            matches!(
                DeclaredWorkerGpuPort::row_observation(&ready, &identity),
                GpuProcessObservation::Matching(_)
            ),
            "a Ready row backs the observed worker"
        );

        for status in [
            ResourceStatus::Pending,
            ResourceStatus::Recovering,
            ResourceStatus::Reconciling,
            ResourceStatus::Deleting,
            ResourceStatus::Failed(DriverFailure::terminal(DriverOp::Reconcile)),
            ResourceStatus::Failed(DriverFailure::retryable(DriverOp::Reconcile)),
        ] {
            assert_eq!(
                DeclaredWorkerGpuPort::row_observation(
                    &worker_row_view([0x33; 16], 7, Some(status.clone())),
                    &identity,
                ),
                GpuProcessObservation::Missing,
                "a non-Ready row ({status:?}) never reads as a live worker"
            );
        }

        assert_eq!(
            DeclaredWorkerGpuPort::row_observation(
                &worker_row_view([0x44; 16], 7, Some(ResourceStatus::Ready)),
                &identity,
            ),
            GpuProcessObservation::StaleIdentity,
            "a row replaced under a new uid is stale, not ambiguous"
        );
    }

    /// The declared row's template is the video posture gate: the video role
    /// exists for two postures (the plain vaapi template and the NVIDIA
    /// decode template the owning Device's `videoNvidiaDecode` setting
    /// selects), so the declared row must be launchable through either,
    /// while no other role may read a video posture as its own row and an
    /// undecodable row must be refused rather than read as a posture.
    #[test]
    fn declared_worker_rows_accept_both_video_postures() {
        let view = |template: &str| {
            let mut view = worker_row_view([0x33; 16], 7, Some(ResourceStatus::Ready));
            view.spec =
                serde_json::to_vec(&serde_json::json!({ "template": template })).expect("spec");
            view
        };

        for (role, template) in [
            (GpuProcessRole::FullGpu, "gpu-worker"),
            (GpuProcessRole::RenderNode, "gpu-render-node"),
            (GpuProcessRole::Video, "video-worker"),
            (GpuProcessRole::Video, "video-worker-nvidia"),
        ] {
            assert_eq!(
                DeclaredWorkerGpuPort::declared_row_template(&view(template), role),
                Ok(template),
                "{role:?} resolves its declared row template"
            );
        }
        assert_eq!(
            DeclaredWorkerGpuPort::declared_row_template(
                &view("video-worker-nvidia"),
                GpuProcessRole::FullGpu,
            ),
            Err(GpuEffectError::SpawnRejected),
            "another role's posture is never this role's row"
        );
        assert_eq!(
            DeclaredWorkerGpuPort::declared_row_template(
                &view("video-worker"),
                GpuProcessRole::Video,
            ),
            Ok("video-worker")
        );
        let mut foreign = worker_row_view([0x33; 16], 7, Some(ResourceStatus::Ready));
        foreign.spec = b"not-json".to_vec();
        assert_eq!(
            DeclaredWorkerGpuPort::declared_row_template(&foreign, GpuProcessRole::Video),
            Err(GpuEffectError::SpawnRejected),
            "an undecodable row is refused, never read as a posture"
        );
    }
}
