//! AudioService and AudioBinding reconciliation through typed ports.

use crate::{
    AudioBindingSpec, AudioGrant, AudioLeaseId, AudioMediator, AudioMediatorError, AudioReadiness,
    GuestAudioReadiness, HostAudioReadiness, MicDecision, MicrophoneArbiter, SpeakerMixer,
    validate_audio_binding, validate_audio_service,
};

/// Closed AudioBinding lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBindingPhase {
    /// Dependencies are still converging.
    Pending,
    /// Both host and guest readiness are established.
    Ready,
    /// A dependency or mediator is temporarily unavailable.
    Degraded,
    /// The binding is being removed.
    Deleted,
}

/// Typed AudioBinding status projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioBindingStatus {
    /// Provider lifecycle phase.
    pub phase: AudioBindingPhase,
    /// Host readiness remains distinct from guest readiness.
    pub host_readiness: HostAudioReadiness,
    /// Guest readiness remains distinct from host readiness.
    pub guest_readiness: GuestAudioReadiness,
    /// Mic arbitration result.
    pub microphone: Option<MicDecision>,
}

/// Typed controller failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioControllerError {
    /// Resource admission failed.
    Admission,
    /// The mediator refused a grant or level.
    Mediator(AudioMediatorError),
}

impl core::fmt::Display for AudioControllerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Admission => "audio-controller-admission-failed",
            Self::Mediator(error) => error.code(),
        })
    }
}

impl std::error::Error for AudioControllerError {}

/// Controller result including separate readiness observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioReconcileResult {
    /// Projected status.
    pub status: AudioBindingStatus,
    /// Whether a host-side effect was attempted.
    pub host_effect_applied: bool,
    /// Whether a guest-side effect was attempted.
    pub guest_effect_applied: bool,
}

/// AudioBinding controller over existing audio policy and mediator ports.
#[derive(Debug)]
pub struct AudioBindingController<M: AudioMediator> {
    mediator: M,
    microphone: MicrophoneArbiter,
    speaker: SpeakerMixer,
}

impl<M: AudioMediator> AudioBindingController<M> {
    /// Construct a controller with bounded arbitration state.
    pub fn new(mediator: M) -> Self {
        Self {
            mediator,
            microphone: MicrophoneArbiter::new(64),
            speaker: SpeakerMixer::new(64),
        }
    }

    /// Borrow the mediator for status or test inspection.
    pub const fn mediator(&self) -> &M {
        &self.mediator
    }

    /// Reconcile one binding without opening a host handle itself.
    pub fn reconcile(
        &mut self,
        binding: &AudioBindingSpec,
        lease: AudioLeaseId,
    ) -> Result<AudioReconcileResult, AudioControllerError> {
        validate_audio_binding(binding).map_err(|_| AudioControllerError::Admission)?;
        let host_readiness = self.mediator.host_readiness();
        let guest_readiness = self.mediator.guest_readiness();
        let mut microphone = None;
        let mut host_effect_applied = false;
        let mut guest_effect_applied = false;

        if binding.grants.mic == AudioGrant::On {
            let decision = self.microphone.request(lease, binding.zone.clone());
            microphone = Some(decision);
            if decision == MicDecision::Granted {
                self.mediator
                    .set_grant(AudioGrant::On)
                    .map_err(AudioControllerError::Mediator)?;
                host_effect_applied = true;
                guest_effect_applied = guest_readiness == GuestAudioReadiness::Ready;
            }
        } else {
            self.microphone.release(lease);
            let _ = self.mediator.set_grant(AudioGrant::Off);
        }
        if binding.grants.speaker == AudioGrant::On {
            self.mediator
                .set_grant(AudioGrant::On)
                .map_err(AudioControllerError::Mediator)?;
            host_effect_applied = true;
        }
        if let Some(level) = binding.grants.speaker_level {
            self.mediator
                .set_level(level)
                .map_err(AudioControllerError::Mediator)?;
            self.speaker
                .set_level(lease, level.get())
                .map_err(|_| AudioControllerError::Admission)?;
            host_effect_applied = true;
        }

        let phase = if self.mediator.readiness() == AudioReadiness::Ready {
            AudioBindingPhase::Ready
        } else {
            AudioBindingPhase::Degraded
        };
        Ok(AudioReconcileResult {
            status: AudioBindingStatus {
                phase,
                host_readiness,
                guest_readiness,
                microphone,
            },
            host_effect_applied,
            guest_effect_applied,
        })
    }

    /// Finalize one binding with mute-before-release ordering.
    pub fn finalize(&mut self, lease: AudioLeaseId) {
        self.microphone.release(lease);
        self.speaker.remove(lease);
        let _ = self.mediator.set_grant(AudioGrant::Off);
    }
}

/// Validate an AudioService before controller registration.
pub fn register_service(service: &crate::AudioServiceSpec) -> Result<(), AudioControllerError> {
    validate_audio_service(service).map_err(|_| AudioControllerError::Admission)
}
