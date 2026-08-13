//! Audio-pipewire Provider contracts and controller-side policy.

#![deny(missing_docs)]

pub mod argv;
mod audio_policy;
pub mod authority;
pub mod controller;
pub mod manifest;
pub mod mediator;
pub mod resource_type;
pub mod telemetry;

pub use argv::{AudioComponentTemplate, AudioTemplateError, RenderedAudioTemplate};
pub use audio_policy::{
    AudioGrant, AudioPolicyError, AudioPolicyState, LevelPercent, LevelPercentError,
    parse_audio_state,
};
pub use authority::{
    AudioAuthorityError, AudioLeaseId, MicDecision, MicrophoneArbiter, SpeakerMixer,
};
pub use controller::{
    AudioBindingController, AudioBindingPhase, AudioBindingStatus, AudioControllerError,
    AudioReconcileResult, register_service,
};
pub use manifest::AudioManifest;
pub use mediator::{
    AudioMediator, AudioMediatorError, AudioReadiness, FakeAudioMediator, GuestAudioReadiness,
    HostAudioReadiness,
};
pub use resource_type::{
    AudioAdmissionError, AudioBindingSpec, AudioGrants, AudioServiceRole, AudioServiceSpec,
    ProviderExtension, validate_audio_binding, validate_audio_binding_in_zone,
    validate_audio_service,
};
