//! Durable Zone-owned reconciliation for AudioService and AudioBinding rows.
//!
//! Audio policy resources are durable store objects.  This module is the
//! daemon-side owner that relists them after restart, validates their
//! relationships, and keeps one controller per binding until finalization.
//! Host effects still flow through the broker-backed mediator in
//! `audio_dispatch`; this registry owns policy state, not privileged handles.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2b_contracts::v3::{ResourceEnvelope, ResourceRef, ResourceTypeName, ZoneId};
use d2b_contracts::{resource_proto as wire, v3::ZoneRevision};
use d2b_provider_audio_pipewire::{
    AudioBindingController, AudioBindingPhase, AudioBindingSpec, AudioBindingStatus,
    AudioControllerError, AudioMediator, AudioServiceRole, AudioServiceSpec, GuestAudioReadiness,
    HostAudioReadiness, MicDecision, shared_microphone_arbiter, validate_audio_binding_in_zone,
    validate_audio_service,
};
use d2b_resource_api::{
    RedbBackend, ResourceApiClient, service::UnavailableUpgradeDispatcher, watch::ResourceWatch,
};
use d2b_resource_store::{
    StoreListRequest, StoreOperationContext, StoreProjection, StoreWatchRequest, StoredResource,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::ServerState;
use crate::audio_dispatch::{DaemonAudioMediator, audio_capability_for_vm};

const AUDIO_SERVICE_TYPE: &str = "audio.d2bus.org.AudioService";
const AUDIO_BINDING_TYPE: &str = "audio.d2bus.org.AudioBinding";
const GUEST_TYPE: &str = "Guest";
type DecodedAudioBinding = (String, (StoredResource, AudioBindingSpec));

/// One relisted audio resource snapshot.
#[derive(Debug, Clone, Default)]
pub(crate) struct AudioResourceSnapshot {
    pub services: Vec<StoredResource>,
    pub bindings: Vec<StoredResource>,
    pub guests: Vec<StoredResource>,
}

/// Stable errors for the daemon-owned audio resource path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioResourceRuntimeError {
    /// A resource body was malformed or used an unexpected provider.
    InvalidResource,
    /// A binding referred to a different or missing Zone resource.
    InvalidRelationship,
    /// A controller finalizer or effect failed.
    Controller(AudioControllerError),
}

impl core::fmt::Display for AudioResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "audio-resource-invalid",
            Self::InvalidRelationship => "audio-resource-relationship-invalid",
            Self::Controller(error) => match error {
                AudioControllerError::Admission => "audio-controller-admission-failed",
                AudioControllerError::Mediator(_) => "audio-controller-effect-failed",
            },
        })
    }
}

impl std::error::Error for AudioResourceRuntimeError {}

/// Daemon-owned status for one durable AudioBinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioBindingRuntimeStatus {
    pub resource: ResourceRef,
    pub status: AudioBindingStatus,
}

pub(crate) fn audio_binding_status_value(status: AudioBindingStatus) -> serde_json::Value {
    serde_json::json!({
        "phase": match status.phase {
            AudioBindingPhase::Pending => "Pending",
            AudioBindingPhase::Ready => "Ready",
            AudioBindingPhase::Degraded => "Degraded",
            AudioBindingPhase::Deleted => "Deleted",
        },
        "hostReadiness": match status.host_readiness {
            HostAudioReadiness::Ready => "Ready",
            HostAudioReadiness::Unavailable => "Unavailable",
        },
        "guestReadiness": match status.guest_readiness {
            GuestAudioReadiness::Ready => "Ready",
            GuestAudioReadiness::Unavailable => "Unavailable",
        },
        "microphone": status.microphone.map(|decision| match decision {
            MicDecision::Granted => "Granted",
            MicDecision::Queued => "Queued",
            MicDecision::QueueFull => "QueueFull",
        }),
    })
}

struct AudioBindingRecord {
    spec: AudioBindingSpec,
    lease: d2b_provider_audio_pipewire::AudioLeaseId,
    controller: Option<AudioBindingController<DaemonAudioMediator>>,
    status: AudioBindingStatus,
}

/// One Zone's durable audio controller registry.
pub(crate) struct AudioResourceRuntime {
    zone: ZoneId,
    state: Arc<ServerState>,
    services: BTreeMap<String, AudioServiceSpec>,
    service_microphones: BTreeMap<String, d2b_provider_audio_pipewire::SharedMicrophoneArbiter>,
    bindings: BTreeMap<String, AudioBindingRecord>,
    pending_finalizers: BTreeMap<String, StoredResource>,
}

impl core::fmt::Debug for AudioResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AudioResourceRuntime")
            .field("zone", &self.zone)
            .field("service_count", &self.services.len())
            .field("service_authority_count", &self.service_microphones.len())
            .field("binding_count", &self.bindings.len())
            .finish()
    }
}

impl AudioResourceRuntime {
    pub(crate) fn new(zone: ZoneId, state: Arc<ServerState>) -> Self {
        Self {
            zone,
            state,
            services: BTreeMap::new(),
            service_microphones: BTreeMap::new(),
            bindings: BTreeMap::new(),
            pending_finalizers: BTreeMap::new(),
        }
    }

    /// Reconcile the complete durable snapshot. Removed bindings are
    /// finalized before service authority is replaced or released.
    pub(crate) fn reconcile(
        &mut self,
        snapshot: AudioResourceSnapshot,
    ) -> Result<(), AudioResourceRuntimeError> {
        let services = decode_services(&self.zone, &snapshot.services)?;
        self.service_microphones
            .retain(|service_ref, _| services.contains_key(service_ref));
        let guests = decode_guest_names(&snapshot.guests)?;
        let bindings = decode_bindings(&self.zone, &snapshot.bindings)?;
        validate_relationships(&services, &bindings, &guests)?;

        let desired_keys = bindings
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<BTreeSet<_>>();
        let removed = self
            .bindings
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in removed {
            let promoted = if let Some(record) = self.bindings.get_mut(&key) {
                if let Some(controller) = record.controller.as_mut() {
                    controller
                        .finalize_shared(record.lease)
                        .map_err(AudioResourceRuntimeError::Controller)?
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(promoted) = promoted {
                self.activate_promoted(promoted)?;
            }
            self.bindings.remove(&key);
        }

        let manifest = crate::load_json::<d2b_core::manifest_v04::ManifestV04>(
            &self.state.config.artifacts.public_manifest_path,
        )
        .ok();

        for (key, (resource, spec)) in bindings {
            if deletion_requested(&resource) {
                let promoted = if let Some(record) = self.bindings.get_mut(&key) {
                    if let Some(controller) = record.controller.as_mut() {
                        controller
                            .finalize_shared(record.lease)
                            .map_err(AudioResourceRuntimeError::Controller)?
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(promoted) = promoted {
                    self.activate_promoted(promoted)?;
                }
                self.bindings.remove(&key);
                if has_audio_finalizer(&resource) {
                    self.pending_finalizers.insert(key.clone(), resource);
                }
                continue;
            }
            let service = services
                .get(&spec.service_ref.to_canonical_string())
                .ok_or(AudioResourceRuntimeError::InvalidRelationship)?;
            if let Some(record) = self.bindings.get_mut(&key)
                && record.spec == spec
                && let Some(controller) = record.controller.as_mut()
            {
                match controller.reconcile(&spec, self.zone.as_str(), record.lease) {
                    Ok(result) => {
                        record.status = result.status;
                    }
                    Err(AudioControllerError::Admission) => {
                        return Err(AudioResourceRuntimeError::InvalidRelationship);
                    }
                    Err(AudioControllerError::Mediator(_)) => {
                        record.status = unavailable_status(
                            AudioBindingPhase::Degraded,
                            controller.mediator().host_readiness(),
                            controller.mediator().guest_readiness(),
                        );
                    }
                }
                continue;
            }
            let promoted = if let Some(old) = self.bindings.get_mut(&key) {
                if let Some(controller) = old.controller.as_mut() {
                    controller
                        .finalize_shared(old.lease)
                        .map_err(AudioResourceRuntimeError::Controller)?
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(promoted) = promoted {
                self.activate_promoted(promoted)?;
            }
            self.bindings.remove(&key);

            let lease = lease_for(&resource.resource_ref);
            let (controller, status) = {
                let capability = manifest
                    .as_ref()
                    .and_then(|manifest| manifest.vms.get(spec.target_ref.name().as_str()))
                    .and_then(audio_capability_for_vm);
                match capability {
                    None => (
                        None,
                        unavailable_status(
                            AudioBindingPhase::Degraded,
                            HostAudioReadiness::Unavailable,
                            GuestAudioReadiness::Unavailable,
                        ),
                    ),
                    Some(capability) => {
                        let capability = if service.service_role == AudioServiceRole::Projection {
                            d2b_core::provider_capabilities::AudioProviderCapability {
                                host_enforcement:
                                    d2b_core::provider_capabilities::AudioHostEnforcementKind::None,
                                ..capability
                            }
                        } else {
                            capability
                        };
                        let mediator = DaemonAudioMediator::new(
                            self.state.as_ref(),
                            spec.target_ref.name().as_str(),
                            capability,
                            d2b_contracts::broker_wire::BrokerCallerRole::AdminUid {
                                uid: self.state.daemon_uid,
                            },
                        );
                        let microphone = self
                            .service_microphones
                            .entry(spec.service_ref.to_canonical_string())
                            .or_insert_with(|| shared_microphone_arbiter(64))
                            .clone();
                        let mut controller =
                            AudioBindingController::with_shared_microphone(mediator, microphone);
                        let result = controller.reconcile(&spec, self.zone.as_str(), lease);
                        match result {
                            Ok(result) => (Some(controller), result.status),
                            Err(AudioControllerError::Admission) => {
                                return Err(AudioResourceRuntimeError::InvalidRelationship);
                            }
                            Err(AudioControllerError::Mediator(_)) => {
                                let mediator = controller.mediator();
                                let host_readiness = mediator.host_readiness();
                                let guest_readiness = mediator.guest_readiness();
                                (
                                    Some(controller),
                                    unavailable_status(
                                        AudioBindingPhase::Degraded,
                                        host_readiness,
                                        guest_readiness,
                                    ),
                                )
                            }
                        }
                    }
                }
            };
            self.bindings.insert(
                key,
                AudioBindingRecord {
                    spec,
                    lease,
                    controller,
                    status,
                },
            );
        }
        self.services = services;
        Ok(())
    }

    fn activate_promoted(
        &mut self,
        lease: d2b_provider_audio_pipewire::AudioLeaseId,
    ) -> Result<(), AudioResourceRuntimeError> {
        let Some(record) = self
            .bindings
            .values_mut()
            .find(|record| record.lease == lease)
        else {
            return Ok(());
        };
        let Some(controller) = record.controller.as_mut() else {
            return Ok(());
        };
        controller
            .activate_promoted_microphone(lease)
            .map_err(AudioResourceRuntimeError::Controller)
    }

    pub(crate) fn statuses(&self) -> Vec<AudioBindingRuntimeStatus> {
        self.bindings
            .iter()
            .filter_map(|(key, record)| {
                ResourceRef::parse(key)
                    .ok()
                    .map(|resource| AudioBindingRuntimeStatus {
                        resource,
                        status: record.status,
                    })
            })
            .collect()
    }

    pub(crate) fn take_pending_finalizers(&mut self) -> Vec<StoredResource> {
        std::mem::take(&mut self.pending_finalizers)
            .into_values()
            .collect()
    }
}

fn unavailable_status(
    phase: AudioBindingPhase,
    host_readiness: HostAudioReadiness,
    guest_readiness: GuestAudioReadiness,
) -> AudioBindingStatus {
    AudioBindingStatus {
        phase,
        host_readiness,
        guest_readiness,
        microphone: None::<MicDecision>,
    }
}

fn lease_for(resource: &ResourceRef) -> d2b_provider_audio_pipewire::AudioLeaseId {
    let digest = Sha256::digest(resource.to_canonical_string().as_bytes());
    let value = u64::from_be_bytes(digest[..8].try_into().expect("fixed digest width"));
    d2b_provider_audio_pipewire::AudioLeaseId::new(value.max(1))
}

fn deletion_requested(resource: &StoredResource) -> bool {
    serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
        .ok()
        .and_then(|value| value.get("metadata").cloned())
        .and_then(|metadata| metadata.get("deletionRequestedAt").cloned())
        .is_some_and(|value| !value.is_null())
}

fn has_audio_finalizer(resource: &StoredResource) -> bool {
    serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
        .ok()
        .and_then(|value| value.get("metadata").cloned())
        .and_then(|metadata| metadata.get("finalizers").cloned())
        .and_then(|value| value.as_array().cloned())
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("audio.d2bus.org/cleanup"))
        })
}

pub(crate) async fn remove_audio_finalizer(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
) -> Result<(), AudioResourceRuntimeError> {
    let mut target = wire::ResourceIdentity::new();
    target.zone = resource.zone.to_canonical_string();
    target.resource_type = resource.resource_ref.resource_type().as_str().to_owned();
    target.name = resource.resource_ref.name().as_str().to_owned();
    target.uid = Some(resource.uid.as_str().to_owned());
    target.generation = Some(resource.generation.get());
    target.revision = Some(resource.revision.get());

    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(resource.revision.get());
    precondition.expected_uid = Some(resource.uid.as_str().to_owned());

    let operation = format!(
        "audio-runtime-finalizer-remove-{}-{}",
        resource.resource_ref.to_canonical_string(),
        resource.revision.get()
    );
    let mut mutation = wire::Mutation::new();
    mutation.kind =
        protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = protobuf::MessageField::some(target);
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation
        .remove_finalizers
        .push("audio.d2bus.org/cleanup".to_owned());
    let mut request = wire::UpdateFinalizersRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation.clone();
    meta.idempotency_key = operation.clone();
    meta.correlation_id = operation.clone();
    meta.trace_id = operation;
    request.meta = protobuf::MessageField::some(meta);
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_finalizers(request).await;
    if response.error.is_some() {
        return Err(AudioResourceRuntimeError::Controller(
            AudioControllerError::Admission,
        ));
    }
    Ok(())
}

fn decode_services(
    zone: &ZoneId,
    resources: &[StoredResource],
) -> Result<BTreeMap<String, AudioServiceSpec>, AudioResourceRuntimeError> {
    let mut services = BTreeMap::new();
    for resource in resources {
        let spec: AudioServiceSpec = decode_spec(resource)?;
        if resource.resource_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
            || resource.zone != *zone
            || validate_audio_service(&spec).is_err()
        {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
        let key = resource.resource_ref.to_canonical_string();
        if services.insert(key, spec).is_some() {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
    }
    Ok(services)
}

fn decode_bindings(
    zone: &ZoneId,
    resources: &[StoredResource],
) -> Result<Vec<DecodedAudioBinding>, AudioResourceRuntimeError> {
    let mut bindings = Vec::new();
    for resource in resources {
        let mut spec: AudioBindingSpec = decode_spec(resource)?;
        spec.zone = zone.as_str().to_owned();
        if resource.zone != *zone {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
        validate_audio_binding_in_zone(&spec, zone.as_str())
            .map_err(|_| AudioResourceRuntimeError::InvalidResource)?;
        let key = resource.resource_ref.to_canonical_string();
        bindings.push((key, (resource.clone(), spec)));
    }
    Ok(bindings)
}

fn decode_guest_names(
    resources: &[StoredResource],
) -> Result<BTreeSet<String>, AudioResourceRuntimeError> {
    resources
        .iter()
        .map(|resource| {
            if resource.resource_ref.resource_type().as_str() != GUEST_TYPE {
                return Err(AudioResourceRuntimeError::InvalidResource);
            }
            Ok(resource.resource_ref.to_canonical_string())
        })
        .collect()
}

fn validate_relationships(
    services: &BTreeMap<String, AudioServiceSpec>,
    bindings: &[(String, (StoredResource, AudioBindingSpec))],
    guests: &BTreeSet<String>,
) -> Result<(), AudioResourceRuntimeError> {
    for (_, (resource, spec)) in bindings {
        if (!deletion_requested(resource)
            && (!services.contains_key(&spec.service_ref.to_canonical_string())
                || !guests.contains(&spec.target_ref.to_canonical_string())))
            || resource.resource_ref.resource_type().as_str() != AUDIO_BINDING_TYPE
        {
            return Err(AudioResourceRuntimeError::InvalidRelationship);
        }
    }
    Ok(())
}

fn decode_spec<T: DeserializeOwned>(
    resource: &StoredResource,
) -> Result<T, AudioResourceRuntimeError> {
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|_| AudioResourceRuntimeError::InvalidResource)?;
    serde_json::from_slice(&envelope.spec().base().to_canonical_bytes())
        .map_err(|_| AudioResourceRuntimeError::InvalidResource)
}

/// Build a store list request for one audio resource type.
pub(crate) fn audio_list_request(
    zone: &ZoneId,
    resource_type: ResourceTypeName,
    suffix: &'static str,
) -> StoreListRequest {
    StoreListRequest {
        operation: StoreOperationContext {
            operation_id: format!("audio-resource-reconcile:{suffix}"),
            idempotency_key: None,
            correlation_id: format!("audio-resource-reconcile:{suffix}"),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![resource_type],
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    }
}

/// Build the one watch that covers all durable audio dependencies.
pub(crate) fn audio_watch_request(zone: &ZoneId) -> StoreWatchRequest {
    StoreWatchRequest {
        operation: StoreOperationContext {
            operation_id: "audio-resource-watch".to_owned(),
            idempotency_key: None,
            correlation_id: "audio-resource-watch".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(AUDIO_SERVICE_TYPE).expect("static audio service type"),
            ResourceTypeName::parse(AUDIO_BINDING_TYPE).expect("static audio binding type"),
            ResourceTypeName::parse(GUEST_TYPE).expect("static guest type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision: ZoneRevision::new(0),
        initial_credits: 64,
        projection: StoreProjection::Full,
    }
}

/// Relist all pages for one resource type before a controller transition.
pub(crate) async fn list_audio_resources(
    store: &d2b_resource_store_redb::RedbResourceStore,
    zone: &ZoneId,
    resource_type: ResourceTypeName,
    suffix: &'static str,
) -> Result<Vec<StoredResource>, AudioResourceRuntimeError> {
    let mut request = audio_list_request(zone, resource_type, suffix);
    let mut resources = Vec::new();
    loop {
        let result = store
            .list(request.clone())
            .await
            .map_err(|_| AudioResourceRuntimeError::InvalidResource)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

/// Run a watch-driven relist loop for a Zone-owned audio registry.
pub(crate) async fn run_audio_watch(
    mut watch: ResourceWatch,
    store: Arc<d2b_resource_store_redb::RedbResourceStore>,
    zone: ZoneId,
    registry: Arc<std::sync::Mutex<Option<AudioResourceRuntime>>>,
    status_client: Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>,
) {
    loop {
        let Some(batch) = watch.recv().await else {
            if watch.resume().await.is_err() {
                return;
            }
            continue;
        };
        let revision = batch.revision();
        let snapshot = match list_audio_snapshot(&store, &zone).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(error = %error, "audio resource relist failed after watch event");
                let _ = watch.acknowledge(revision).await;
                continue;
            }
        };
        let binding_resources = snapshot.bindings.clone();
        let (statuses, pending_finalizers) = match registry.lock() {
            Ok(mut slot) => match slot.as_mut() {
                Some(runtime) => match runtime.reconcile(snapshot) {
                    Ok(()) => (Some(runtime.statuses()), runtime.take_pending_finalizers()),
                    Err(error) => {
                        tracing::warn!(error = %error, "audio resource reconciliation degraded");
                        (None, Vec::new())
                    }
                },
                None => (None, Vec::new()),
            },
            Err(_) => return,
        };
        for resource in pending_finalizers {
            if let Err(error) = remove_audio_finalizer(status_client.as_ref(), &resource).await {
                tracing::warn!(
                    error = %error,
                    finalizer = "audio",
                    "audio finalizer removal failed"
                );
            }
        }
        if let Some(statuses) = statuses {
            for status in statuses {
                let Some(resource) = binding_resources
                    .iter()
                    .find(|resource| resource.resource_ref == status.resource)
                else {
                    continue;
                };
                if let Err(error) = crate::resource_runtime::persist_resource_status(
                    status_client.as_ref(),
                    resource,
                    &audio_binding_status_value(status.status),
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        "audio status projection persistence failed"
                    );
                }
            }
        }
        if watch.acknowledge(revision).await.is_err() && watch.resume().await.is_err() {
            return;
        }
    }
}

pub(crate) async fn list_audio_snapshot(
    store: &d2b_resource_store_redb::RedbResourceStore,
    zone: &ZoneId,
) -> Result<AudioResourceSnapshot, AudioResourceRuntimeError> {
    let services = list_audio_resources(
        store,
        zone,
        ResourceTypeName::parse(AUDIO_SERVICE_TYPE).expect("static audio service type"),
        "service",
    )
    .await?;
    let bindings = list_audio_resources(
        store,
        zone,
        ResourceTypeName::parse(AUDIO_BINDING_TYPE).expect("static audio binding type"),
        "binding",
    )
    .await?;
    let guests = list_audio_resources(
        store,
        zone,
        ResourceTypeName::parse(GUEST_TYPE).expect("static guest type"),
        "guest",
    )
    .await?;
    Ok(AudioResourceSnapshot {
        services,
        bindings,
        guests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_lease_identity_is_stable_and_nonzero() {
        let resource = ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap();
        assert_eq!(lease_for(&resource), lease_for(&resource));
        assert_ne!(
            lease_for(&resource),
            lease_for(&ResourceRef::parse("audio.d2bus.org.AudioBinding/other").unwrap())
        );
    }

    #[test]
    fn projection_status_never_claims_host_readiness() {
        let status = unavailable_status(
            AudioBindingPhase::Degraded,
            HostAudioReadiness::Unavailable,
            GuestAudioReadiness::Unavailable,
        );
        assert_eq!(status.phase, AudioBindingPhase::Degraded);
        assert_eq!(status.host_readiness, HostAudioReadiness::Unavailable);
        assert_eq!(status.guest_readiness, GuestAudioReadiness::Unavailable);
    }

    #[test]
    fn audio_status_projection_is_stable_and_separates_readiness() {
        let status = audio_binding_status_value(unavailable_status(
            AudioBindingPhase::Degraded,
            HostAudioReadiness::Ready,
            GuestAudioReadiness::Unavailable,
        ));
        assert_eq!(status["phase"], "Degraded");
        assert_eq!(status["hostReadiness"], "Ready");
        assert_eq!(status["guestReadiness"], "Unavailable");
        assert!(status["microphone"].is_null());
    }

    #[test]
    fn watch_covers_service_binding_and_guest_dependencies() {
        let zone = ZoneId::parse("dev").unwrap();
        let request = audio_watch_request(&zone);
        assert_eq!(request.zone, zone);
        assert_eq!(request.initial_credits, 64);
        assert_eq!(
            request
                .resource_types
                .iter()
                .map(ResourceTypeName::as_str)
                .collect::<Vec<_>>(),
            vec![AUDIO_SERVICE_TYPE, AUDIO_BINDING_TYPE, GUEST_TYPE]
        );
    }

    #[test]
    fn relationship_validation_rejects_missing_guest_and_cross_service() {
        let zone = ZoneId::parse("dev").unwrap();
        let service_ref = ResourceRef::parse("audio.d2bus.org.AudioService/owner").unwrap();
        let guest_ref = ResourceRef::parse("Guest/vm").unwrap();
        let binding_ref = ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap();
        let service =
            AudioServiceSpec::owner(ResourceRef::parse("Endpoint/audio").unwrap(), zone.as_str())
                .unwrap();
        let binding =
            AudioBindingSpec::new(service_ref.clone(), guest_ref.clone(), zone.as_str()).unwrap();
        let resource = StoredResource {
            resource_ref: binding_ref.clone(),
            zone: zone.clone(),
            uid: d2b_contracts::v3::ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .unwrap(),
            generation: d2b_contracts::v3::ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json: br#"{"metadata":{}}"#.to_vec(),
            payload_digest: String::new(),
        };
        let bindings = vec![(
            resource.resource_ref.to_canonical_string(),
            (resource, binding),
        )];
        let mut services = BTreeMap::new();
        services.insert(service_ref.to_canonical_string(), service);
        assert_eq!(
            validate_relationships(&services, &bindings, &BTreeSet::new()),
            Err(AudioResourceRuntimeError::InvalidRelationship)
        );
        assert_eq!(
            validate_relationships(
                &services,
                &bindings,
                &BTreeSet::from([guest_ref.to_canonical_string()])
            ),
            Ok(())
        );
        let mut deleting_resource = bindings[0].1.0.clone();
        deleting_resource.canonical_json =
            br#"{"metadata":{"deletionRequestedAt":"2026-08-15T00:00:00Z"}}"#.to_vec();
        let deleting_bindings = vec![(
            binding_ref.to_canonical_string(),
            (deleting_resource, bindings[0].1.1.clone()),
        )];
        assert_eq!(
            validate_relationships(&BTreeMap::new(), &deleting_bindings, &BTreeSet::new()),
            Ok(())
        );
    }
}
