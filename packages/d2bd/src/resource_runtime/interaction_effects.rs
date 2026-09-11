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
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceEnvelope, ResourceGeneration,
    ResourceRef, ZoneId, ZoneRevision, canonical_digest,
};
use d2b_provider_audio_pipewire::{AudioBindingPhase, AudioBindingSpec};
use d2b_provider_display_wayland::WaylandSessionSpec;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::{ResourceSelector, ResourceView};
use d2b_resource_runtime::ResourceStatus;
use d2b_resource_store::{
    ResourceAssignmentScope, StoreErrorKind, StoreGetRequest, StoreOperationContext,
    StoreProjection, StoredResource,
};
use serde_json::{Value, json};

use super::ZoneResourceRuntime;
use crate::ServerState;
use crate::audio_resource_runtime::{
    AUDIO_BINDING_TYPE, AudioResourceRuntime, AudioResourceRuntimeError, audio_binding_projection,
};
use crate::interaction_driver::{
    InteractionDriverEffects, InteractionEffectError, InteractionEffectOutcome,
    InteractionEffectPhase, InteractionEffectRequest, InteractionFinalize, InteractionKind,
    key_ref, resource_uid, shell_pool_spec, shell_session_execution, shell_session_pool_ref,
};
use crate::resource_plane_v3::{PlaneRoute, route_resource_type};

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

    fn store_get(&self, target: &ResourceRef, operation_id: &str) -> StoreGetRequest {
        StoreGetRequest {
            operation: StoreOperationContext {
                operation_id: operation_id.to_owned(),
                idempotency_key: None,
                correlation_id: operation_id.to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: self.zone.clone(),
            target: target.clone(),
            expected_uid: None,
            projection: StoreProjection::Full,
        }
    }

    /// The manager view of one converted row (`None` for unconverted types).
    async fn live_view(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<ResourceView>, InteractionEffectError> {
        if route_resource_type(target.resource_type().as_str()) != PlaneRoute::NewPlane {
            return Ok(None);
        }
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

    /// The live phase of one resource: the manager view for converted rows
    /// (a converted row's actor status is the only status there is, R11), the
    /// durable row's `/status/phase` for unconverted rows.
    async fn live_phase(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<&'static str>, InteractionEffectError> {
        if let Some(view) = self.live_view(target).await? {
            return Ok(Some(view_phase(&view)));
        }
        if route_resource_type(target.resource_type().as_str()) == PlaneRoute::NewPlane {
            // Converted row absent from the manager: nothing to read.
            return Ok(None);
        }
        let runtime = self.runtime()?;
        match runtime
            .store()
            .get(self.store_get(target, "interaction-phase"))
            .await
        {
            Ok(resource) => Ok(phase_of_json(&resource.canonical_json)),
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => Ok(None),
            Err(_) => Err(InteractionEffectError::Unavailable),
        }
    }

    async fn is_ready(&self, target: &ResourceRef) -> Result<bool, InteractionEffectError> {
        Ok(matches!(self.live_phase(target).await?, Some("Ready")))
    }

    /// The authoritative row of one resource: the manager view re-rendered
    /// as a durable envelope for converted rows, the durable row itself for
    /// unconverted rows.
    async fn live_stored(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<StoredResource>, InteractionEffectError> {
        if let Some(view) = self.live_view(target).await? {
            return Ok(Some(stored_from_view(&self.zone, &view)?));
        }
        if route_resource_type(target.resource_type().as_str()) == PlaneRoute::NewPlane {
            return Ok(None);
        }
        let runtime = self.runtime()?;
        match runtime
            .store()
            .get(self.store_get(target, "interaction-read"))
            .await
        {
            Ok(resource) => Ok(Some(resource)),
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => Ok(None),
            Err(_) => Err(InteractionEffectError::Unavailable),
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

    /// Spec documents of every row of one ResourceType: the manager list for
    /// converted rows, the durable store for unconverted rows.
    async fn specs_of_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<Value>, InteractionEffectError> {
        if route_resource_type(resource_type) == PlaneRoute::NewPlane {
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
            return views
                .iter()
                .map(|view| spec_document_value(&view.spec))
                .collect();
        }
        let runtime = self.runtime()?;
        runtime
            .committed_resources_of_type(resource_type)
            .await
            .map(|resources| {
                resources
                    .iter()
                    .map(envelope_spec_document)
                    .collect::<Vec<_>>()
            })
            .map_err(|_| InteractionEffectError::Unavailable)
    }

    /// One authoritative audio dependency read (old `fresh_audio_dependency`):
    /// a missing row fails closed, and a persisted assignment fence must
    /// stay consistent with the row it fences.
    async fn fresh_audio_dependency(
        &self,
        target: &ResourceRef,
    ) -> Result<StoredResource, InteractionEffectError> {
        let Some(authoritative) = self.live_stored(target).await? else {
            return Err(InteractionEffectError::InvalidResource);
        };
        validate_audio_dependency_identity(&authoritative, target, &self.zone)?;
        self.validate_audio_assignment(&authoritative).await?;
        Ok(authoritative)
    }

    /// Re-check one persisted assignment fence against its row (old
    /// `validate_audio_assignment`, row-consistency half). The old
    /// controller-identity half compared the Runner's assigned role and
    /// generations; the v3 plane has no per-resource Runner identity, so the
    /// row-consistency fence is what remains and it still fails closed.
    async fn validate_audio_assignment(
        &self,
        resource: &StoredResource,
    ) -> Result<(), InteractionEffectError> {
        let runtime = self.runtime()?;
        let Some(assignment) = runtime
            .store()
            .assignment_fence(self.zone.clone(), resource.resource_ref.clone())
            .await
            .map_err(|_| InteractionEffectError::Unavailable)?
        else {
            return Ok(());
        };
        if assignment.resource_uid != resource.uid
            || assignment.resource_revision != resource.revision
            || assignment.provider_generation.get() == 0
            || assignment.controller_generation.get() == 0
            || assignment.session_generation.get() == 0
            || !matches!(assignment.scope, ResourceAssignmentScope::Primary)
        {
            return Err(InteractionEffectError::InvalidResource);
        }
        Ok(())
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
        self.validate_audio_assignment(&target).await?;
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
        let service = self.fresh_audio_dependency(&spec.service_ref).await?;
        let guest = self.fresh_audio_dependency(&spec.target_ref).await?;
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

fn view_phase(view: &ResourceView) -> &'static str {
    if view.deleting {
        return "Deleted";
    }
    if view.status_generation.is_some() && view.status_generation != Some(view.generation) {
        // A status published before the committed generation is not observed
        // state of the current row: fail closed.
        return "Pending";
    }
    match view.status {
        Some(ResourceStatus::Ready) => "Ready",
        Some(ResourceStatus::Failed(_)) => "Failed",
        Some(ResourceStatus::Deleting) => "Deleted",
        _ => "Pending",
    }
}

fn phase_of_json(bytes: &[u8]) -> Option<&'static str> {
    serde_json::from_slice::<Value>(bytes)
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
        })
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
/// controller registry and the assignment fence consume): the authored
/// metadata plus the store-authoritative identity (KTD2/KTD8: the row owns
/// uid, generation, and wire revision).
fn stored_from_view(
    zone: &ZoneId,
    view: &ResourceView,
) -> Result<StoredResource, InteractionEffectError> {
    let spec = spec_document_value(&view.spec)?;
    let metadata: Value = if view.metadata.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&view.metadata)
            .map_err(|_| InteractionEffectError::InvalidResource)?
    };
    let uid = resource_uid(&view.uid).map_err(|_| InteractionEffectError::InvalidResource)?;
    let envelope = json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": view.key.type_name,
        "metadata": {
            "annotations": metadata.get("annotations").cloned().unwrap_or_else(|| json!({})),
            "createdAt": "1970-01-01T00:00:00.000Z",
            "deletionRequestedAt": if view.deleting {
                Value::String("1970-01-01T00:00:00.000Z".to_owned())
            } else {
                Value::Null
            },
            "finalizers": [],
            "generation": view.generation,
            "labels": metadata.get("labels").cloned().unwrap_or_else(|| json!({})),
            "name": view.key.name,
            "ownerRef": metadata.get("ownerRef").cloned().unwrap_or(Value::Null),
            "revision": view.generation,
            "uid": uid.as_str(),
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "zone": zone.as_str(),
        },
        "spec": spec,
        "status": {
            "observedGeneration": view.generation,
            "phase": view_phase(view),
        },
    });
    let bytes =
        serde_json::to_vec(&envelope).map_err(|_| InteractionEffectError::InvalidResource)?;
    let canonical = CanonicalJsonValue::parse(&bytes)
        .map_err(|_| InteractionEffectError::InvalidResource)?
        .to_canonical_bytes();
    let payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    Ok(StoredResource {
        resource_ref: key_ref(&view.key),
        zone: zone.clone(),
        uid,
        owner_uid: None,
        owner_generation: None,
        generation: ResourceGeneration::new(view.generation)
            .map_err(|_| InteractionEffectError::InvalidResource)?,
        revision: ZoneRevision::new(view.generation),
        canonical_json: canonical,
        payload_digest,
    })
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

