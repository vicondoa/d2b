//! Durable Zone-owned reconciliation for AudioService and AudioBinding rows.
//!
//! Audio policy resources are durable store objects. This module is the
//! family's owner that reconciles fresh per-resource snapshots, validates
//! their relationships, and keeps one controller per binding until
//! finalization. It is the moved daemon-side registry
//! (`audio_resource_runtime.rs`), now served by the family's effects: host
//! effects still flow through the broker-backed mediator behind the
//! [`crate::facets::AudioMediatorSource`] facet; the registry owns policy
//! state, not privileged handles.
//!
//! One registry lives per Zone behind the declared facet set, so every
//! effects value built from the same facets - the six drivers' shared port
//! and the hosted service - reconciles the same controller state.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use d2b_contracts_resource::v3::{ResourceEnvelope, ResourceRef, StoredResource, ZoneId};
use d2b_provider_audio_pipewire::{
    AudioArbitrationState, AudioBindingController, AudioBindingPhase, AudioBindingSpec,
    AudioBindingStatus, AudioControllerError, AudioEnforcementPosture, AudioGrant,
    AudioLastSetApplied, AudioMediator, AudioServiceRole, AudioServiceSpec, GuestAudioReadiness,
    HostAudioReadiness, MicDecision, resource_type::PROVIDER_REF, shared_microphone_arbiter,
    validate_audio_binding_in_zone, validate_audio_service,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::facets::AudioMediatorSource;
use crate::vocabulary::{AUDIO_BINDING_TYPE, AUDIO_SERVICE_TYPE};

const GUEST_TYPE: &str = "Guest";

/// Stable errors for the daemon-owned audio resource path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AudioResourceRuntimeError {
    /// A resource body was malformed or used an unexpected provider.
    InvalidResource,
    /// A wire spec or envelope failed to parse; carries the serde reason.
    InvalidSpec(String),
    /// A binding referred to a different or missing Zone resource.
    InvalidRelationship,
    /// A controller finalizer or effect failed.
    Controller(AudioControllerError),
}

impl core::fmt::Display for AudioResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource | Self::InvalidSpec(_) => "audio-resource-invalid",
            Self::InvalidRelationship => "audio-resource-relationship-invalid",
            Self::Controller(error) => match error {
                AudioControllerError::Admission => "audio-controller-admission-failed",
                AudioControllerError::Mediator(_) => "audio-controller-effect-failed",
            },
        })
    }
}

impl std::error::Error for AudioResourceRuntimeError {}

/// Family-owned status for one durable AudioBinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBindingRuntimeStatus {
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
        "channels": {
            "speaker": {
                "grant": status.channels.speaker.grant.as_wire_str(),
                "level": status.channels.speaker.level,
                "liveEnforced": status.channels.speaker.live_enforced,
            },
            "mic": {
                "grant": status.channels.mic.grant.as_wire_str(),
                "gain": status.channels.mic.gain,
                "liveEnforced": status.channels.mic.live_enforced,
                "arbitrationState": match status.channels.mic.arbitration_state {
                    AudioArbitrationState::Inactive => "inactive",
                    AudioArbitrationState::Queued => "queued",
                    AudioArbitrationState::Active => "active",
                    AudioArbitrationState::Blocked => "blocked",
                },
            },
        },
        "enforcementPosture": match status.enforcement_posture {
            AudioEnforcementPosture::HostAndGuest => "HostAndGuest",
            AudioEnforcementPosture::HostOnly => "HostOnly",
            AudioEnforcementPosture::GuestOnly => "GuestOnly",
            AudioEnforcementPosture::None => "None",
        },
        "lastSetApplied": match status.last_set_applied {
            AudioLastSetApplied::HostAndGuest => "HostAndGuest",
            AudioLastSetApplied::HostOnly => "HostOnly",
            AudioLastSetApplied::GuestOnly => "GuestOnly",
            AudioLastSetApplied::NotApplied => "NotApplied",
        },
    })
}

struct AudioBindingRecord {
    spec: AudioBindingSpec,
    lease: d2b_provider_audio_pipewire::AudioLeaseId,
    controller: Option<AudioBindingController<Box<dyn AudioMediator>>>,
    status: AudioBindingStatus,
}

/// One Zone's durable audio controller registry.
pub(crate) struct AudioResourceRuntime {
    zone: ZoneId,
    audio: Arc<dyn AudioMediatorSource>,
    services: BTreeMap<String, AudioServiceSpec>,
    service_microphones: BTreeMap<String, d2b_provider_audio_pipewire::SharedMicrophoneArbiter>,
    bindings: BTreeMap<String, AudioBindingRecord>,
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
    pub(crate) fn new(zone: ZoneId, audio: Arc<dyn AudioMediatorSource>) -> Self {
        Self {
            zone,
            audio,
            services: BTreeMap::new(),
            service_microphones: BTreeMap::new(),
            bindings: BTreeMap::new(),
        }
    }

    /// Reconcile one authored AudioService without touching sibling rows.
    pub(crate) fn reconcile_service_resource(
        &mut self,
        resource: &StoredResource,
    ) -> Result<(), AudioResourceRuntimeError> {
        if resource.zone != self.zone
            || resource.resource_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
            || !is_audio_resource(resource, &self.zone)?
        {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
        let spec: AudioServiceSpec = decode_spec(resource)?;
        validate_audio_service(&spec)
            .map_err(|_| AudioResourceRuntimeError::InvalidResource)?;
        let key = resource.resource_ref.to_canonical_string();
        if deletion_requested(resource) {
            if self
                .bindings
                .values()
                .any(|record| record.spec.service_ref == resource.resource_ref)
            {
                return Err(AudioResourceRuntimeError::InvalidRelationship);
            }
            self.services.remove(&key);
            self.service_microphones.remove(&key);
        } else {
            self.services.insert(key, spec);
        }
        Ok(())
    }

    /// Reconcile one authored AudioBinding against its exact fresh
    /// AudioService and Guest dependencies.
    pub(crate) fn reconcile_binding_resource(
        &mut self,
        resource: &StoredResource,
        service: &StoredResource,
        guest: &StoredResource,
    ) -> Result<Option<AudioBindingRuntimeStatus>, AudioResourceRuntimeError> {
        if resource.zone != self.zone
            || resource.resource_ref.resource_type().as_str() != AUDIO_BINDING_TYPE
            || !is_audio_resource(resource, &self.zone)?
        {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
        let mut spec: AudioBindingSpec = decode_spec(resource)?;
        spec.zone = self.zone.as_str().to_owned();
        validate_audio_binding_in_zone(&spec, self.zone.as_str())
            .map_err(|_| AudioResourceRuntimeError::InvalidResource)?;
        if deletion_requested(resource) {
            return Err(AudioResourceRuntimeError::InvalidRelationship);
        }
        if service.resource_ref != spec.service_ref
            || service.zone != self.zone
            || service.resource_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
            || deletion_requested(service)
            || !is_audio_resource(service, &self.zone)?
        {
            return Err(AudioResourceRuntimeError::InvalidRelationship);
        }
        let service_spec: AudioServiceSpec = decode_spec(service)?;
        validate_audio_service(&service_spec)
            .map_err(|_| AudioResourceRuntimeError::InvalidRelationship)?;
        if guest.resource_ref != spec.target_ref
            || guest.zone != self.zone
            || guest.resource_ref.resource_type().as_str() != GUEST_TYPE
            || deletion_requested(guest)
            || ResourceEnvelope::from_json(&guest.canonical_json).is_err()
        {
            return Err(AudioResourceRuntimeError::InvalidRelationship);
        }

        self.services
            .insert(service.resource_ref.to_canonical_string(), service_spec.clone());
        let key = resource.resource_ref.to_canonical_string();
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
            return Ok(Some(AudioBindingRuntimeStatus {
                resource: resource.resource_ref.clone(),
                status: record.status,
            }));
        }

        let promoted = if let Some(old) = self.bindings.get_mut(&key) {
            if let Some(controller) = old.controller.as_mut() {
                controller
                    .finalize(old.lease)
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
        // The mediator source is the daemon's broker-backed construction: the
        // target capability row and the host enforcement paths live behind the
        // declared facet, so the family carries no manifest or daemon state
        // (U12). A target with no audio capability publishes the degraded,
        // host-and-guest unavailable status exactly as the daemon registry
        // did.
        let projection = service_spec.service_role == AudioServiceRole::Projection;
        let (controller, status) = match self
            .audio
            .build(spec.target_ref.name().as_str(), projection)
        {
            None => (
                None,
                unavailable_status(
                    AudioBindingPhase::Degraded,
                    HostAudioReadiness::Unavailable,
                    GuestAudioReadiness::Unavailable,
                ),
            ),
            Some(mediator) => {
                let microphone = self
                    .service_microphones
                    .entry(spec.service_ref.to_canonical_string())
                    .or_insert_with(|| {
                        shared_microphone_arbiter(NonZeroUsize::new(64).expect("fixed bound"))
                    })
                    .clone();
                let mut controller =
                    AudioBindingController::with_shared_microphone(mediator, microphone);
                match controller.reconcile(&spec, self.zone.as_str(), lease) {
                    Ok(result) => (Some(controller), result.status),
                    Err(AudioControllerError::Admission) => {
                        return Err(AudioResourceRuntimeError::InvalidRelationship);
                    }
                    Err(AudioControllerError::Mediator(_)) => {
                        let (host_readiness, guest_readiness) = {
                            let mediator = controller.mediator();
                            (mediator.host_readiness(), mediator.guest_readiness())
                        };
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
        Ok(Some(AudioBindingRuntimeStatus {
            resource: resource.resource_ref.clone(),
            status,
        }))
    }

    /// Remove one deleting AudioBinding from the in-memory authority.
    pub(crate) fn finalize_binding_resource(
        &mut self,
        resource: &StoredResource,
    ) -> Result<(), AudioResourceRuntimeError> {
        if resource.zone != self.zone
            || resource.resource_ref.resource_type().as_str() != AUDIO_BINDING_TYPE
            || !is_audio_resource(resource, &self.zone)?
        {
            return Err(AudioResourceRuntimeError::InvalidResource);
        }
        let key = resource.resource_ref.to_canonical_string();
        let promoted = if let Some(record) = self.bindings.get_mut(&key) {
            if let Some(controller) = record.controller.as_mut() {
                controller
                    .finalize(record.lease)
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
}

/// The per-zone shared handle the effects lock to reconcile the family's
/// audio policy state.
pub(crate) struct AudioEffectRegistry {
    inner: tokio::sync::Mutex<AudioResourceRuntime>,
}

impl AudioEffectRegistry {
    pub(crate) fn new(zone: ZoneId, audio: Arc<dyn AudioMediatorSource>) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(AudioResourceRuntime::new(zone, audio)),
        }
    }

    pub(crate) async fn reconcile_service(
        &self,
        resource: &StoredResource,
    ) -> Result<(), AudioResourceRuntimeError> {
        self.inner.lock().await.reconcile_service_resource(resource)
    }

    pub(crate) async fn reconcile_binding(
        &self,
        resource: &StoredResource,
        service: &StoredResource,
        guest: &StoredResource,
    ) -> Result<Option<AudioBindingRuntimeStatus>, AudioResourceRuntimeError> {
        self.inner
            .lock()
            .await
            .reconcile_binding_resource(resource, service, guest)
    }

    pub(crate) async fn finalize_binding(
        &self,
        resource: &StoredResource,
    ) -> Result<(), AudioResourceRuntimeError> {
        self.inner.lock().await.finalize_binding_resource(resource)
    }

    /// The zone's current binding statuses, for the hosted service surface.
    pub(crate) async fn statuses(&self) -> Vec<AudioBindingRuntimeStatus> {
        self.inner.lock().await.statuses()
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
        channels: d2b_provider_audio_pipewire::AudioBindingChannels {
            speaker: d2b_provider_audio_pipewire::AudioSpeakerStatus {
                grant: AudioGrant::Off,
                level: None,
                live_enforced: false,
            },
            mic: d2b_provider_audio_pipewire::AudioMicrophoneStatus {
                grant: AudioGrant::Off,
                gain: None,
                live_enforced: false,
                arbitration_state: AudioArbitrationState::Inactive,
            },
        },
        enforcement_posture: AudioEnforcementPosture::None,
        last_set_applied: AudioLastSetApplied::NotApplied,
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

fn is_audio_resource(
    resource: &StoredResource,
    zone: &ZoneId,
) -> Result<bool, AudioResourceRuntimeError> {
    if resource.resource_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
        && resource.resource_ref.resource_type().as_str() != AUDIO_BINDING_TYPE
    {
        return Err(AudioResourceRuntimeError::InvalidResource);
    }
    if resource.zone != *zone {
        return Err(AudioResourceRuntimeError::InvalidResource);
    }
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|error| AudioResourceRuntimeError::InvalidSpec(error.to_string()))?;
    Ok(envelope
        .spec()
        .provider_ref()
        .is_some_and(|provider| provider.to_canonical_string() == PROVIDER_REF))
}

fn decode_spec<T: DeserializeOwned>(
    resource: &StoredResource,
) -> Result<T, AudioResourceRuntimeError> {
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|error| AudioResourceRuntimeError::InvalidSpec(error.to_string()))?;
    let mut spec = serde_json::to_value(envelope.spec().base())
        .map_err(|error| AudioResourceRuntimeError::InvalidSpec(error.to_string()))?;
    if let Some(provider_ref) = envelope.spec().provider_ref() {
        let object = spec
            .as_object_mut()
            .ok_or(AudioResourceRuntimeError::InvalidResource)?;
        object.insert(
            "providerRef".to_owned(),
            serde_json::Value::String(provider_ref.to_canonical_string()),
        );
    }
    serde_json::from_value(spec)
        .map_err(|error| AudioResourceRuntimeError::InvalidSpec(error.to_string()))
}

/// The `status. resource` projection for one AudioBinding (old
/// `audio_binding_status_projection_with_status`): the typed channel status
/// plus the realized Process/Endpoint references the driver owns.
pub(crate) fn audio_binding_projection(
    spec: &AudioBindingSpec,
    realization_refs: &[ResourceRef],
    status: &AudioBindingStatus,
) -> serde_json::Value {
    let typed_status = audio_binding_status_value(*status);
    serde_json::json!({
        "channels": typed_status["channels"],
        "enforcementPosture": typed_status["enforcementPosture"],
        "lastSetApplied": typed_status["lastSetApplied"],
        "observedServiceRef": spec.service_ref.to_canonical_string(),
        "realizationRefs": realization_refs
            .iter()
            .map(|reference| reference.to_canonical_string())
            .collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod audio_registry_tests;
