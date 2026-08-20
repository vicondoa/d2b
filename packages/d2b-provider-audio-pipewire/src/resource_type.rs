//! Strict audio-pipewire ResourceType admission.

use d2b_contracts_zone_session::v3::ResourceRef;
use serde::{Deserialize, Serialize};

/// The only implementation Provider admitted for audio resources.
pub const PROVIDER_REF: &str = "Provider/audio-pipewire";
const AUDIO_SERVICE_TYPE: &str = "audio.d2bus.org.AudioService";

/// Service role in the provider-neutral AudioService type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioServiceRole {
    /// Owns the local AudioMediator and authority.
    Owner,
    /// Core-generated projection backed by a ResourceImport.
    Projection,
}

/// Strict implementation extension kept out of the provider-neutral base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderExtension {
    capture_alias: String,
}

impl ProviderExtension {
    /// Construct a bounded provider-only setting.
    pub fn new(capture_alias: impl Into<String>) -> Self {
        Self {
            capture_alias: capture_alias.into(),
        }
    }

    /// Borrow the private capture alias.
    pub fn capture_alias(&self) -> &str {
        &self.capture_alias
    }
}

/// Provider-neutral AudioService desired state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioServiceSpec {
    /// Provider identity.
    pub provider_ref: String,
    /// Owner or projection role.
    pub service_role: AudioServiceRole,
    /// Same-Zone private implementation Endpoints.
    pub implementation_endpoint_refs: Vec<ResourceRef>,
    /// Provider-neutral operations exposed by the service.
    pub operations: Vec<String>,
    /// Internal Zone context used by the admission adapter. Resource wire
    /// envelopes carry Zone in metadata rather than in `spec`.
    #[serde(skip)]
    pub zone: String,
    /// Optional provider extension. It is rejected by base admission unless
    /// consumed through the signed provider envelope.
    #[serde(skip)]
    provider_extension: Option<ProviderExtension>,
}

impl AudioServiceSpec {
    /// Construct an owner Service with one local authority Endpoint.
    pub fn owner(
        endpoint_ref: ResourceRef,
        zone: impl Into<String>,
    ) -> Result<Self, AudioAdmissionError> {
        if endpoint_ref.resource_type().as_str() != "Endpoint" {
            return Err(AudioAdmissionError::EndpointType);
        }
        Ok(Self {
            provider_ref: PROVIDER_REF.to_owned(),
            service_role: AudioServiceRole::Owner,
            implementation_endpoint_refs: vec![endpoint_ref],
            operations: vec!["playback".to_owned(), "capture".to_owned()],
            zone: zone.into(),
            provider_extension: None,
        })
    }

    /// Construct a Core-generated projection Service.
    pub fn projection(zone: impl Into<String>) -> Result<Self, AudioAdmissionError> {
        Ok(Self {
            provider_ref: PROVIDER_REF.to_owned(),
            service_role: AudioServiceRole::Projection,
            implementation_endpoint_refs: Vec::new(),
            operations: vec!["playback".to_owned(), "capture".to_owned()],
            zone: zone.into(),
            provider_extension: None,
        })
    }
}

/// Provider-neutral AudioBinding desired state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioBindingSpec {
    /// Provider identity.
    pub provider_ref: String,
    /// Same-Zone AudioService reference.
    pub service_ref: ResourceRef,
    /// Guest target.
    pub target_ref: ResourceRef,
    /// Internal Zone context used by the admission adapter. Resource wire
    /// envelopes carry Zone in metadata rather than in `spec`.
    #[serde(skip)]
    pub zone: String,
    /// Durable grant state.
    pub grants: AudioGrants,
    /// Implementation extensions are never accepted in the base.
    #[serde(skip)]
    provider_extension: Option<ProviderExtension>,
}

impl AudioBindingSpec {
    /// Construct a binding for one Guest and same-Zone Service.
    pub fn new(
        service_ref: ResourceRef,
        target_ref: ResourceRef,
        zone: impl Into<String>,
    ) -> Result<Self, AudioAdmissionError> {
        if service_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
            || target_ref.resource_type().as_str() != "Guest"
        {
            return Err(AudioAdmissionError::ReferenceType);
        }
        Ok(Self {
            provider_ref: PROVIDER_REF.to_owned(),
            service_ref,
            target_ref,
            zone: zone.into(),
            grants: AudioGrants::default(),
            provider_extension: None,
        })
    }

    /// Attach a provider extension for negative admission tests.
    #[must_use]
    pub fn with_provider_extension(mut self, extension: ProviderExtension) -> Self {
        self.provider_extension = Some(extension);
        self
    }
}

/// Bounded provider-neutral audio grant fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioGrants {
    /// Microphone grant.
    pub mic: crate::AudioGrant,
    /// Speaker grant.
    pub speaker: crate::AudioGrant,
    /// Optional speaker level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_level: Option<crate::LevelPercent>,
    /// Optional microphone gain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_gain: Option<crate::LevelPercent>,
}

impl Default for AudioGrants {
    fn default() -> Self {
        let state = crate::AudioPolicyState::default_v2();
        Self {
            mic: state.mic,
            speaker: state.speaker,
            speaker_level: state.speaker_level,
            mic_gain: state.mic_gain,
        }
    }
}

/// Audio admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAdmissionError {
    /// Provider reference was not the canonical Provider.
    ProviderRef,
    /// Resource reference had the wrong type.
    ReferenceType,
    /// Endpoint had the wrong type.
    EndpointType,
    /// Projection carried owner-only state.
    ProjectionOwnerState,
    /// Base spec attempted to carry provider-only fields.
    ProviderFieldInBase,
    /// Service and target were not in the same Zone.
    CrossZone,
}

impl core::fmt::Display for AudioAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::ProviderRef => "audio-provider-ref-invalid",
            Self::ReferenceType => "audio-reference-type-invalid",
            Self::EndpointType => "audio-endpoint-type-invalid",
            Self::ProjectionOwnerState => "audio-projection-owner-state",
            Self::ProviderFieldInBase => "audio-provider-field-in-base",
            Self::CrossZone => "audio-cross-zone",
        })
    }
}

impl std::error::Error for AudioAdmissionError {}

/// Validate an AudioService base object.
pub fn validate_audio_service(spec: &AudioServiceSpec) -> Result<(), AudioAdmissionError> {
    if spec.provider_ref != PROVIDER_REF {
        return Err(AudioAdmissionError::ProviderRef);
    }
    if spec.provider_extension.is_some() {
        return Err(AudioAdmissionError::ProviderFieldInBase);
    }
    match spec.service_role {
        AudioServiceRole::Owner => {
            if spec.implementation_endpoint_refs.len() != 1
                || spec.implementation_endpoint_refs[0]
                    .resource_type()
                    .as_str()
                    != "Endpoint"
            {
                return Err(AudioAdmissionError::EndpointType);
            }
        }
        AudioServiceRole::Projection if !spec.implementation_endpoint_refs.is_empty() => {
            return Err(AudioAdmissionError::ProjectionOwnerState);
        }
        AudioServiceRole::Projection => {}
    }
    Ok(())
}

/// Validate an AudioBinding base object.
pub fn validate_audio_binding(spec: &AudioBindingSpec) -> Result<(), AudioAdmissionError> {
    if spec.provider_ref != PROVIDER_REF {
        return Err(AudioAdmissionError::ProviderRef);
    }
    if spec.service_ref.resource_type().as_str() != AUDIO_SERVICE_TYPE
        || spec.target_ref.resource_type().as_str() != "Guest"
    {
        return Err(AudioAdmissionError::ReferenceType);
    }
    if spec.provider_extension.is_some() {
        return Err(AudioAdmissionError::ProviderFieldInBase);
    }
    Ok(())
}

/// Validate the required same-Zone Service/Binding relationship.
pub fn validate_audio_binding_in_zone(
    spec: &AudioBindingSpec,
    service_zone: &str,
) -> Result<(), AudioAdmissionError> {
    validate_audio_binding(spec)?;
    if !spec.zone.is_empty() && spec.zone != service_zone {
        return Err(AudioAdmissionError::CrossZone);
    }
    Ok(())
}
