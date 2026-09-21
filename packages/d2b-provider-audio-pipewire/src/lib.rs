//! Audio-pipewire Provider contracts and controller-side policy.

#![deny(missing_docs)]

pub mod argv;
pub mod authority;
pub mod controller;
pub mod manifest;
pub mod mediator;
pub mod resource_type;
#[allow(missing_docs)]
pub mod state;
pub mod telemetry;

/// The PipeWire runtime socket file name the host capability probe looks for.
///
/// The name is shared by contract: the host family's probe asserts the same
/// kernel-visible socket name under the caller's runtime dir, spelled in
/// `d2b-provider-host` rather than imported (the probe needs no sibling
/// crate); this crate declares the vocabulary for its own family surfaces.
pub const PIPEWIRE_RUNTIME_SOCKET: &str = "pipewire-0";

pub use argv::{AudioComponentTemplate, AudioTemplateError, RenderedAudioTemplate};
pub use d2b_contracts::audio::{
    AudioGrant, AudioPolicyError, AudioPolicyState, LevelPercent, LevelPercentError,
    parse_audio_state,
};
pub use authority::{
    AudioAuthorityError, AudioLeaseId, MicDecision, MicrophoneArbiter, SharedMicrophoneArbiter,
    SpeakerMixer, shared_microphone_arbiter,
};
pub use controller::{
    AudioArbitrationState, AudioBindingChannels, AudioBindingController, AudioBindingPhase,
    AudioBindingStatus, AudioControllerError, AudioEnforcementPosture, AudioLastSetApplied,
    AudioMicrophoneStatus, AudioReconcileResult, AudioReconcileResultWithChildren,
    AudioSpeakerStatus, AUDIO_REPAIR_INTERVAL_SECS, register_service,
};
pub use manifest::AudioManifest;
pub use mediator::{
    AudioChannel, AudioMediator, AudioMediatorError, AudioReadiness, FakeAudioMediator,
    GuestAudioReadiness, HostAudioReadiness,
};
pub use resource_type::{
    AudioAdmissionError, AudioBindingSpec, AudioGrants, AudioServiceRole, AudioServiceSpec,
    PROVIDER_REF, ProviderExtension, validate_audio_binding, validate_audio_binding_in_zone,
    validate_audio_service,
};
pub use state::{
    AudioStateIoError, AudioStateLock, acquire_audio_state_lock, audio_lock_path, audio_state_path,
    read_audio_state_locked, read_audio_state_unlocked, write_audio_state_locked,
    write_audio_state_unlocked,
};
