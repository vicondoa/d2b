//! Production effects for the U12 interaction and shell drivers.
//!
//! Every typed Provider effect the U9 driver family dispatches lives here:
//! the display session admission against the committed interaction identity,
//! the audio-pipewire controller registry, and the shell pool/session
//! reference checks. The port is the dyn-erased [`InteractionDriverEffects`]
//! boundary; the daemon owns every side effect behind it and the drivers own
//! the child rows.
//!
//! Live readiness is read through the manager view for converted rows (a
//! converted row's actor status is the only status there is, R11) and through
//! the durable row for unconverted rows - the same split the old effects got
//! from `/status/phase`. The driver never sees either.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceEnvelope, ResourceRef, StoredResource, ZoneId};
use d2b_provider_audio_pipewire::{AudioBindingPhase, AudioBindingSpec};
use d2b_provider_display_wayland::WaylandSessionSpec;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::{ResourceSelector, ResourceView};
use d2b_resource_runtime::ResourceStatus;
use serde_json::Value;

use super::ZoneResourceRuntime;
use crate::ServerState;
use crate::audio_resource_runtime::{
    AUDIO_BINDING_TYPE, AudioResourceRuntime, AudioResourceRuntimeError, audio_binding_projection,
};
use crate::interaction_driver::{
    InteractionDriverEffects, InteractionEffectError, InteractionEffectOutcome,
    InteractionEffectPhase, InteractionEffectRequest, InteractionFinalize, InteractionKind,
    key_ref, shell_pool_spec, shell_session_execution, shell_session_pool_ref,
};

/// Production composition adapter for the closed interaction/shell family.
///
/// The adapter performs the Provider-owned typed admission before any effect
/// call. A missing live broker/resource binding is returned as a retryable
/// refusal; it is never converted into generic convergence.
pub(crate) struct ProductionInteractionDriverEffects {
    state: Arc<ServerState>,
    zone: ZoneId,
}

impl ProductionInteractionDriverEffects {
    pub(crate) fn new(state: Arc<ServerState>, zone: ZoneId) -> Self {
        Self { state, zone }
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, InteractionEffectError> {
        self.state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(InteractionEffectError::Unavailable)
    }

    fn plane(
        &self,
    ) -> Result<Arc<crate::resource_plane_v3::ResourcePlaneV3>, InteractionEffectError> {
        self.runtime()?
            .v3_plane()
            .map_err(|_| InteractionEffectError::Unavailable)
    }

    /// The manager view of one row.
    async fn live_view(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<ResourceView>, InteractionEffectError> {
        let plane = self.plane()?;
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        plane
            .client()
            .get(key)
            .await
            .map_err(|_| InteractionEffectError::Unavailable)
    }

    /// The live phase of one resource from the manager view: the row actor's
    /// status is the only status there is (R11).
    async fn live_phase(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<&'static str>, InteractionEffectError> {
        Ok(self.live_view(target).await?.map(|view| view_phase(&view)))
    }

    async fn is_ready(&self, target: &ResourceRef) -> Result<bool, InteractionEffectError> {
        Ok(matches!(self.live_phase(target).await?, Some("Ready")))
    }

    /// The authoritative row of one resource: the manager view re-rendered
    /// as a durable envelope.
    async fn live_stored(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<StoredResource>, InteractionEffectError> {
        match self.live_view(target).await? {
            Some(view) => Ok(Some(stored_from_view(&view)?)),
            None => Ok(None),
        }
    }

    /// Whether every expected child is owned this pass and live-Ready.
    async fn children_ready(
        &self,
        request: &InteractionEffectRequest<'_>,
        expected: &[ResourceRef],
    ) -> Result<bool, InteractionEffectError> {
        for target in expected {
            if !request
                .children
                .iter()
                .any(|child| child.resource_ref == *target)
            {
                return Ok(false);
            }
            if !self.is_ready(target).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Spec documents of every row of one ResourceType, from the manager.
    async fn specs_of_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<Value>, InteractionEffectError> {
        let plane = self.plane()?;
        let views = plane
            .client()
            .list(ResourceSelector {
                zone: Some(self.zone.as_str().to_owned()),
                type_name: Some(resource_type.to_owned()),
                owner: None,
            })
            .await
            .map_err(|_| InteractionEffectError::Unavailable)?;
        views
            .iter()
            .map(|view| spec_document_value(&view.spec))
            .collect()
    }

    /// One authoritative audio dependency read (old `fresh_audio_dependency`).
    ///
    /// The row itself is the fence: the persisted assignment-fence re-check
    /// the old runner-era path did has no durable fence to compare against
    /// since U14 retired the store, so the row identity is what validates. A
    /// dependency the manager does not hold *yet* reads `None` and the caller
    /// defers instead of failing the binding terminally: canonical bundle
    /// order commits an `AudioBinding` before the `AudioService` it names
    /// (bundle order is (type, name) and `AudioBinding` sorts first), and an
    /// API-created binding can precede its service entirely.
    async fn fresh_audio_dependency(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<StoredResource>, InteractionEffectError> {
        audio_dependency_row(self.live_stored(target).await?, target, &self.zone)
    }

    async fn reconcile_display_session(
        &self,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        let spec: WaylandSessionSpec = serde_json::from_value(request.spec.clone())
            .map_err(|_| InteractionEffectError::InvalidResource)?;
        let runtime = self.runtime()?;
        let identity = runtime
            .interaction_identity()
            .ok_or(InteractionEffectError::Unavailable)?;
        let session_ref = key_ref(&request.target);
        if identity.wayland_session_ref() != &session_ref
            || identity.wayland_session_uid() != &request.uid
            || identity.subject_ref() != spec.guest_ref()
            || identity.host_execution_ref() != spec.host_ref()
            || identity.user_ref() != spec.user_ref()
        {
            return Err(InteractionEffectError::InvalidResource);
        }
        for dependency in [
            spec.guest_ref(),
            spec.host_ref(),
            spec.user_ref(),
            spec.policy_ref(),
        ] {
            if !self.is_ready(dependency).await? {
                return Ok(InteractionEffectOutcome::phase(
                    InteractionEffectPhase::Pending,
                ));
            }
        }
        let intents = crate::interaction_composition::display_owned_child_intents(
            &self.zone,
            &session_ref,
            &request.uid,
            &spec,
            request.generation,
            request.controller_generation,
        )
        .map_err(|_| InteractionEffectError::InvalidResource)?;
        let expected = intents
            .iter()
            .map(|intent| intent.target().clone())
            .collect::<Vec<_>>();
        if !self.children_ready(request, &expected).await? {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        }
        Ok(InteractionEffectOutcome::projection(
            InteractionEffectPhase::Ready,
            display_projection(&intents, request),
        ))
    }

    async fn reconcile_audio_service(
        &self,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        let Some(target) = self.live_stored(&key_ref(&request.target)).await? else {
            return Err(InteractionEffectError::Unavailable);
        };
        let runtime = self.runtime()?;
        let mut audio = runtime
            .audio_runtime
            .lock()
            .map_err(|_| InteractionEffectError::Unavailable)?;
        let registry = audio.get_or_insert_with(|| {
            AudioResourceRuntime::new(self.zone.clone(), Arc::clone(&self.state))
        });
        registry
            .reconcile_service_resource(&target)
            .map_err(map_audio_effect_error)?;
        Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Ready))
    }

    async fn reconcile_audio_binding(
        &self,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        let spec: AudioBindingSpec = spec_with_provider_ref(&request.spec, request.provider_ref.as_ref())
            .and_then(|spec| {
                serde_json::from_value(spec).map_err(|_| InteractionEffectError::InvalidResource)
            })?;
        let Some(target) = self.live_stored(&key_ref(&request.target)).await? else {
            return Err(InteractionEffectError::Unavailable);
        };
        let Some(service) = self.fresh_audio_dependency(&spec.service_ref).await? else {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        };
        let Some(guest) = self.fresh_audio_dependency(&spec.target_ref).await? else {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        };
        let service_value =
            serde_json::from_slice::<Value>(&service.canonical_json).unwrap_or_default();
        let guest_value =
            serde_json::from_slice::<Value>(&guest.canonical_json).unwrap_or_default();
        if resource_phase(&service_value) != Some("Ready")
            || resource_phase(&guest_value) != Some("Ready")
        {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        }
        let status = {
            let runtime = self.runtime()?;
            let mut audio = runtime
                .audio_runtime
                .lock()
                .map_err(|_| InteractionEffectError::Unavailable)?;
            let registry = audio.get_or_insert_with(|| {
                AudioResourceRuntime::new(self.zone.clone(), Arc::clone(&self.state))
            });
            registry
                .reconcile_binding_resource(&target, &service, &guest)
                .map_err(map_audio_effect_error)?
                .ok_or(InteractionEffectError::InvalidResource)?
        };
        let children = d2b_provider_audio_pipewire::AudioBindingController::<
            crate::audio_dispatch::DaemonAudioMediator,
        >::child_resources(&key_ref(&request.target), &spec)
        .map_err(|_| InteractionEffectError::InvalidResource)?;
        let expected = children
            .iter()
            .map(|intent| intent.resource_ref().clone())
            .collect::<Vec<_>>();
        if !self.children_ready(request, &expected).await? {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        }
        let phase = if status.status.phase == AudioBindingPhase::Ready {
            InteractionEffectPhase::Ready
        } else {
            InteractionEffectPhase::Pending
        };
        let realization_refs = request
            .children
            .iter()
            .filter(|child| {
                matches!(
                    child.resource_ref.resource_type().as_str(),
                    "Process" | "EphemeralProcess" | "Endpoint"
                )
            })
            .map(|child| child.resource_ref.clone())
            .collect::<Vec<_>>();
        Ok(InteractionEffectOutcome::projection(
            phase,
            audio_binding_projection(&spec, &realization_refs, &status.status),
        ))
    }

    async fn reconcile_shell_pool(
        &self,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        let provider_ref = request.provider_ref.as_ref().map(ResourceRef::to_canonical_string);
        let (execution_ref, user_ref) =
            shell_pool_spec(&request.spec, provider_ref.as_deref())?;
        let phase = if self.is_ready(&execution_ref).await? && self.is_ready(&user_ref).await? {
            InteractionEffectPhase::Ready
        } else {
            InteractionEffectPhase::Pending
        };
        Ok(InteractionEffectOutcome::phase(phase))
    }

    async fn reconcile_shell_session(
        &self,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        let provider_ref = request.provider_ref.as_ref().map(ResourceRef::to_canonical_string);
        let (execution_ref, user_ref) =
            shell_session_execution(&request.spec, provider_ref.as_deref())?;
        let user_ref = user_ref.ok_or(InteractionEffectError::InvalidResource)?;
        let pool_ref = shell_session_pool_ref(&request.spec, provider_ref.as_deref())?;
        let Some(pool) = self.live_stored(&pool_ref).await? else {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        };
        if resource_phase(&serde_json::from_slice::<Value>(&pool.canonical_json).unwrap_or_default())
            != Some("Ready")
        {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        }
        if spec_ref_at(&pool.canonical_json, "/spec/executionRef")? != execution_ref
            || spec_ref_at(&pool.canonical_json, "/spec/userRef")? != user_ref
        {
            return Err(InteractionEffectError::InvalidResource);
        }
        let expected = request
            .children
            .iter()
            .map(|child| child.resource_ref.clone())
            .collect::<Vec<_>>();
        if !self.children_ready(request, &expected).await? {
            return Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Pending,
            ));
        }
        Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Ready))
    }
}

#[async_trait]
impl InteractionDriverEffects for ProductionInteractionDriverEffects {
    async fn reconcile(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        match kind {
            // The policy envelope is the whole contract: a decoded policy
            // realizes no children and no Provider effect.
            InteractionKind::DisplayWaylandPolicy => Ok(InteractionEffectOutcome::phase(
                InteractionEffectPhase::Ready,
            )),
            InteractionKind::DisplayWaylandSession => {
                self.reconcile_display_session(request).await
            }
            InteractionKind::AudioService => self.reconcile_audio_service(request).await,
            InteractionKind::AudioBinding => self.reconcile_audio_binding(request).await,
            InteractionKind::ShellPool => self.reconcile_shell_pool(request).await,
            InteractionKind::ShellSession => self.reconcile_shell_session(request).await,
        }
    }

    async fn finalize(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError> {
        match kind {
            InteractionKind::DisplayWaylandPolicy
            | InteractionKind::DisplayWaylandSession
            | InteractionKind::ShellSession => Ok(InteractionFinalize::Complete),
            // An AudioService refuses to go away while a Binding still
            // references it (old `finalize_u9`): the owner stays with its
            // durable deleting mark and the pass retries.
            InteractionKind::AudioService => {
                let target = key_ref(&request.target).to_canonical_string();
                let dangling = self.specs_of_type(AUDIO_BINDING_TYPE).await?.iter().any(
                    |binding| {
                        binding.pointer("/serviceRef").and_then(Value::as_str)
                            == Some(target.as_str())
                    },
                );
                Ok(if dangling {
                    InteractionFinalize::Pending
                } else {
                    InteractionFinalize::Complete
                })
            }
            InteractionKind::AudioBinding => {
                let Some(target) = self.live_stored(&key_ref(&request.target)).await? else {
                    return Ok(InteractionFinalize::Complete);
                };
                let runtime = self.runtime()?;
                let mut audio = runtime
                    .audio_runtime
                    .lock()
                    .map_err(|_| InteractionEffectError::Unavailable)?;
                let registry = audio.get_or_insert_with(|| {
                    AudioResourceRuntime::new(self.zone.clone(), Arc::clone(&self.state))
                });
                registry
                    .finalize_binding_resource(&target)
                    .map_err(map_audio_effect_error)?;
                Ok(InteractionFinalize::Complete)
            }
            // A ShellPool refuses to go away while a Session still
            // references it (old `finalize_u9`).
            InteractionKind::ShellPool => {
                let target = key_ref(&request.target).to_canonical_string();
                let dangling = self
                    .specs_of_type("shell-terminal.d2bus.org.ShellSession")
                    .await?
                    .iter()
                    .any(|session| {
                        session.pointer("/poolRef").and_then(Value::as_str)
                            == Some(target.as_str())
                    });
                Ok(if dangling {
                    InteractionFinalize::Pending
                } else {
                    InteractionFinalize::Complete
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Envelope rendering and projections
// ---------------------------------------------------------------------------

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

fn resource_phase(value: &Value) -> Option<&str> {
    value.pointer("/status/phase").and_then(Value::as_str)
}

/// The spec document behind either persisted representation: the compiled
/// spec for Nix rows, the `spec` member for API rows that persist the full
/// envelope minus status.
fn spec_document_value(bytes: &[u8]) -> Result<Value, InteractionEffectError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| InteractionEffectError::InvalidResource)?;
    Ok(envelope_spec_document(&value))
}

fn envelope_spec_document(value: &Value) -> Value {
    if value.get("apiVersion").is_some() && value.get("spec").is_some() {
        value.get("spec").cloned().unwrap_or_default()
    } else {
        value.clone()
    }
}

/// The base spec with `spec.providerRef` re-inserted (the typed audio specs
/// carry the field; old `AudioResourceRuntime::decode_spec` re-inserted it
/// after the envelope split).
fn spec_with_provider_ref(
    base: &Value,
    provider_ref: Option<&ResourceRef>,
) -> Result<Value, InteractionEffectError> {
    let mut spec = base.clone();
    if let Some(provider_ref) = provider_ref {
        let object = spec
            .as_object_mut()
            .ok_or(InteractionEffectError::InvalidResource)?;
        object.insert(
            "providerRef".to_owned(),
            Value::String(provider_ref.to_canonical_string()),
        );
    }
    Ok(spec)
}

fn spec_ref_at(bytes: &[u8], path: &str) -> Result<ResourceRef, InteractionEffectError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| InteractionEffectError::InvalidResource)?;
    value
        .pointer(path)
        .and_then(Value::as_str)
        .and_then(|reference| ResourceRef::parse(reference).ok())
        .ok_or(InteractionEffectError::InvalidResource)
}

/// Re-render one manager view as a durable envelope (the shape the audio
/// controller registry and the assignment fence consume).
///
/// The manager-backed projection owns this rendering: handing the view to
/// `manager_row_stored` serves exactly the envelope the new plane publishes
/// (store-authoritative identity per KTD2/KTD8, the strict-contract metadata
/// and status), so the strict readers here can never drift from the public
/// surface's shape.
fn stored_from_view(view: &ResourceView) -> Result<StoredResource, InteractionEffectError> {
    d2b_resource_api::manager_backend::manager_row_stored(view)
        .map_err(|_| InteractionEffectError::InvalidResource)
}

/// Classify one fresh audio dependency read: a row the manager does not hold
/// yet reads `None` - the caller defers with `Pending`, because the
/// dependency may legitimately land after the binding - while a committed row
/// is returned once its identity validates. A committed row that is not the
/// named dependency stays `InvalidResource`: retrying cannot make a foreign
/// row this binding's dependency.
fn audio_dependency_row(
    live: Option<StoredResource>,
    target: &ResourceRef,
    zone: &ZoneId,
) -> Result<Option<StoredResource>, InteractionEffectError> {
    let Some(authoritative) = live else {
        return Ok(None);
    };
    validate_audio_dependency_identity(&authoritative, target, zone)?;
    Ok(Some(authoritative))
}

/// Whether one dependency row is authentic and still owned by this Zone
/// (old `validate_audio_dependency_identity`).
fn validate_audio_dependency_identity(
    resource: &StoredResource,
    target: &ResourceRef,
    zone: &ZoneId,
) -> Result<(), InteractionEffectError> {
    if resource.zone != *zone
        || resource.resource_ref != *target
        || resource.uid.as_str().is_empty()
        || resource.generation.get() == 0
        || resource.revision.get() == 0
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|_| InteractionEffectError::InvalidResource)?;
    let metadata = envelope.metadata();
    if metadata.zone() != zone
        || metadata.uid() != &resource.uid
        || metadata.generation() != resource.generation
        || metadata.revision() != resource.revision
        || envelope
            .digest()
            .map_err(|_| InteractionEffectError::InvalidResource)?
            != resource.payload_digest
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok(())
}

fn map_audio_effect_error(error: AudioResourceRuntimeError) -> InteractionEffectError {
    match error {
        AudioResourceRuntimeError::InvalidResource
        | AudioResourceRuntimeError::InvalidRelationship => {
            InteractionEffectError::InvalidResource
        }
        AudioResourceRuntimeError::Controller(_) => InteractionEffectError::Unavailable,
    }
}

/// The old `status.resource` projection for a display session (old
/// `display_resource_projection`): the two worker Process references and the
/// private Endpoint with its committed generation.
fn display_projection(
    intents: &[d2b_core_controller::OwnedChildIntent],
    request: &InteractionEffectRequest<'_>,
) -> Value {
    let process_refs = intents
        .iter()
        .filter(|intent| intent.target().resource_type().as_str() == "Process")
        .map(|intent| intent.target().to_canonical_string())
        .collect::<Vec<_>>();
    let endpoint_ref = intents
        .iter()
        .find(|intent| intent.target().resource_type().as_str() == "Endpoint")
        .map(|intent| intent.target().clone());
    let endpoint_generation = endpoint_ref.as_ref().and_then(|target| {
        request
            .children
            .iter()
            .find(|child| child.resource_ref == *target)
            .map(|child| child.generation)
    });
    let resource = d2b_provider_display_wayland::WaylandSessionResourceStatus {
        proxy_process_ref: process_refs
            .first()
            .and_then(|reference| ResourceRef::parse(reference).ok()),
        guest_frontend_process_ref: process_refs
            .get(1)
            .and_then(|reference| ResourceRef::parse(reference).ok()),
        wayland_endpoint_ref: endpoint_ref,
        wayland_endpoint_generation: endpoint_generation,
        policy_digest: String::new(),
    };
    crate::interaction_composition::wayland_session_resource_projection(&resource)
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{ResourceRef, StoredResource, ZoneId};
    use d2b_resource_runtime::error::{DriverFailure, DriverOp};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;

    use super::{audio_dependency_row, stored_from_view, view_phase, InteractionEffectError};

    /// Regression (P2): an absent audio dependency must defer, not fail the
    /// binding terminally. Canonical bundle order commits an `AudioBinding`
    /// before the `AudioService` it names (bundle order is (type, name)), and
    /// an API-created binding can precede its service entirely; the pre-fix
    /// read answered `InvalidResource` for absence, which the driver maps to
    /// the terminal `SpecInvalid` refusal - a refusal schedules no requeue,
    /// so the binding stayed `Failed` forever. The read now defers while the
    /// row is missing, progresses once the row appears, and keeps the
    /// terminal identity refusal for a committed row that is not the named
    /// dependency.
    #[test]
    fn absent_audio_dependency_defers_until_the_row_appears() {
        let zone = ZoneId::parse("work").expect("zone");
        let service_ref =
            ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").expect("service ref");

        // Absent: `None`, and `reconcile_audio_binding` returns the Pending
        // phase (a requeued pass), never an error.
        assert_eq!(
            audio_dependency_row(None, &service_ref, &zone),
            Ok(None),
            "an uncommitted dependency must defer, not refuse",
        );

        // The row appearing is progress: the validated row is handed to the
        // phase checks.
        let service = dependency_row("work", "audio.d2bus.org.AudioService", "host-audio");
        assert_eq!(
            audio_dependency_row(Some(service.clone()), &service_ref, &zone),
            Ok(Some(service.clone())),
            "a committed dependency must progress the binding",
        );

        // A committed row that is not the named dependency stays terminal:
        // another name, another Zone, and another type each refuse.
        let other_name = dependency_row("work", "audio.d2bus.org.AudioService", "other-service");
        let other_zone = dependency_row("other", "audio.d2bus.org.AudioService", "host-audio");
        let other_type = dependency_row("work", "Guest", "host-audio");
        for foreign in [other_name, other_zone, other_type] {
            assert_eq!(
                audio_dependency_row(Some(foreign), &service_ref, &zone),
                Err(InteractionEffectError::InvalidResource),
                "a committed row that is not the named dependency stays terminal",
            );
        }
    }

    /// One manager dependency view rendered through the canonical projection:
    /// the row shape `live_stored` hands the audio dependency read.
    fn dependency_row(zone: &str, resource_type: &str, name: &str) -> StoredResource {
        stored_from_view(&ResourceView {
            key: ResourceKey::new(zone, resource_type, name),
            uid: [0x51; 16],
            generation: 3,
            deleting: false,
            provenance: ResourceProvenance::Nix,
            spec: serde_json::to_vec(&serde_json::json!({"providerRef": "Provider/audio-pipewire"}))
                .expect("dependency spec"),
            metadata: Vec::new(),
            owner_key: None,
            status: Some(ResourceStatus::Ready),
            status_generation: Some(3),
            status_projection: None,
        })
        .expect("dependency row")
    }

    /// One manager view carrying the given status classification, published
    /// generation, and durable deleting mark.
    fn phase_view(
        status: Option<ResourceStatus>,
        status_generation: Option<u64>,
        deleting: bool,
    ) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", "Endpoint", "phase-view"),
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

    /// Issue #515: `view_phase` delegates to the canonical wire producer.
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

