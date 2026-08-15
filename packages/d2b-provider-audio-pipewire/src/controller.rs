//! AudioService and AudioBinding reconciliation through typed ports.

use crate::{
    AudioBindingSpec, AudioChannel, AudioGrant, AudioLeaseId, AudioMediator, AudioMediatorError,
    AudioReadiness, GuestAudioReadiness, HostAudioReadiness, MicDecision, SharedMicrophoneArbiter,
    SpeakerMixer, validate_audio_binding_in_zone, validate_audio_service,
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
    microphone: SharedMicrophoneArbiter,
    activate_promoted: bool,
    microphone_effect_applied: bool,
    speaker: SpeakerMixer,
}

impl<M: AudioMediator> AudioBindingController<M> {
    /// Construct a controller with bounded arbitration state.
    pub fn new(mediator: M) -> Self {
        Self {
            mediator,
            microphone: crate::shared_microphone_arbiter(64),
            activate_promoted: true,
            microphone_effect_applied: false,
            speaker: SpeakerMixer::new(64),
        }
    }

    /// Construct a controller sharing one AudioService microphone authority.
    pub fn with_shared_microphone(mediator: M, microphone: SharedMicrophoneArbiter) -> Self {
        Self {
            mediator,
            microphone,
            activate_promoted: false,
            microphone_effect_applied: false,
            speaker: SpeakerMixer::new(64),
        }
    }

    /// Borrow the mediator for status or test inspection.
    pub const fn mediator(&self) -> &M {
        &self.mediator
    }

    /// Return the active microphone lease for status and recovery.
    pub fn active_microphone_lease(&self) -> Option<AudioLeaseId> {
        match self.microphone.lock() {
            Ok(arbiter) => arbiter.active(),
            Err(poisoned) => poisoned.into_inner().active(),
        }
    }

    /// Reconcile one binding without opening a host handle itself.
    pub fn reconcile(
        &mut self,
        binding: &AudioBindingSpec,
        service_zone: &str,
        lease: AudioLeaseId,
    ) -> Result<AudioReconcileResult, AudioControllerError> {
        validate_audio_binding_in_zone(binding, service_zone)
            .map_err(|_| AudioControllerError::Admission)?;
        let host_readiness = self.mediator.host_readiness();
        let guest_readiness = self.mediator.guest_readiness();
        let mut microphone = None;
        let mut host_effect_applied = false;
        let mut guest_effect_applied = false;

        if binding.grants.mic == AudioGrant::On {
            let already_active = self.active_microphone_lease() == Some(lease);
            let decision = match self.microphone.lock() {
                Ok(mut arbiter) => arbiter.request(lease),
                Err(poisoned) => poisoned.into_inner().request(lease),
            };
            microphone = Some(decision);
            let needs_effect = decision == MicDecision::Granted
                && (!already_active || !self.microphone_effect_applied);
            if needs_effect {
                self.mediator
                    .set_channel_grant(AudioChannel::Microphone, AudioGrant::On)
                    .map_err(|error| {
                        if !already_active {
                            match self.microphone.lock() {
                                Ok(mut arbiter) => {
                                    arbiter.release(lease);
                                }
                                Err(poisoned) => {
                                    poisoned.into_inner().release(lease);
                                }
                            }
                        } else {
                            match self.microphone.lock() {
                                Ok(mut arbiter) => arbiter.requeue_active(lease),
                                Err(poisoned) => poisoned.into_inner().requeue_active(lease),
                            }
                        }
                        AudioControllerError::Mediator(error)
                    })?;
                self.microphone_effect_applied = true;
                host_effect_applied = true;
                guest_effect_applied = guest_readiness == GuestAudioReadiness::Ready;
            }
        } else {
            self.release_microphone(lease)?;
        }
        if binding.grants.speaker == AudioGrant::On {
            let transition = self
                .speaker
                .set_grant(lease, true)
                .map_err(|_| AudioControllerError::Admission)?;
            if transition {
                if let Err(error) = self
                    .mediator
                    .set_channel_grant(AudioChannel::Speaker, AudioGrant::On)
                {
                    let _ = self.speaker.set_grant(lease, false);
                    return Err(AudioControllerError::Mediator(error));
                }
                host_effect_applied = true;
            }
        } else if self.speaker.has_grant(lease) {
            let last = self.speaker.is_last_grant(lease);
            if last {
                self.mediator
                    .set_channel_grant(AudioChannel::Speaker, AudioGrant::Off)
                    .map_err(AudioControllerError::Mediator)?;
            }
            self.speaker
                .set_grant(lease, false)
                .map_err(|_| AudioControllerError::Admission)?;
        }
        if let Some(level) = binding.grants.speaker_level {
            self.speaker
                .can_set_level(lease, level.get())
                .map_err(|_| AudioControllerError::Admission)?;
            if self.speaker.level(lease) != Some(level.get()) {
                self.mediator
                    .set_channel_level(AudioChannel::Speaker, level)
                    .map_err(AudioControllerError::Mediator)?;
            }
            self.speaker
                .set_level(lease, level.get())
                .map_err(|_| AudioControllerError::Admission)?;
            host_effect_applied = true;
        }

        let phase = match microphone {
            Some(MicDecision::Queued) => AudioBindingPhase::Pending,
            Some(MicDecision::QueueFull) => AudioBindingPhase::Degraded,
            _ if self.mediator.readiness() == AudioReadiness::Ready => AudioBindingPhase::Ready,
            _ => AudioBindingPhase::Degraded,
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
    pub fn finalize(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        self.finalize_inner(lease)
    }

    /// Finalize a binding whose microphone authority is shared with other
    /// controllers.
    ///
    /// The next lease is returned but is not enabled through this binding's
    /// mediator. The daemon reconciles the promoted binding so the effect is
    /// applied to the correct target.
    pub fn finalize_shared(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        self.finalize_inner(lease)
    }

    /// Apply the microphone effect for a lease promoted by another shared
    /// controller's finalization.
    pub fn activate_promoted_microphone(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<(), AudioControllerError> {
        if self.active_microphone_lease() != Some(lease) {
            return Ok(());
        }
        if let Err(error) = self
            .mediator
            .set_channel_grant(AudioChannel::Microphone, AudioGrant::On)
        {
            match self.microphone.lock() {
                Ok(mut arbiter) => arbiter.requeue_active(lease),
                Err(poisoned) => poisoned.into_inner().requeue_active(lease),
            }
            return Err(AudioControllerError::Mediator(error));
        }
        self.microphone_effect_applied = true;
        Ok(())
    }

    fn finalize_inner(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        let promoted = self.release_microphone(lease)?;
        if self.speaker.is_last_grant(lease) {
            self.mediator
                .set_channel_grant(AudioChannel::Speaker, AudioGrant::Off)
                .map_err(AudioControllerError::Mediator)?;
        }
        self.speaker.remove(lease);
        Ok(promoted)
    }

    fn release_microphone(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        if self.active_microphone_lease() != Some(lease) {
            match self.microphone.lock() {
                Ok(mut arbiter) => {
                    arbiter.release(lease);
                }
                Err(poisoned) => {
                    poisoned.into_inner().release(lease);
                }
            }
            return Ok(None);
        }
        self.mediator
            .set_channel_grant(AudioChannel::Microphone, AudioGrant::Off)
            .map_err(AudioControllerError::Mediator)?;
        self.microphone_effect_applied = false;
        let next = match self.microphone.lock() {
            Ok(mut arbiter) => {
                arbiter.release(lease);
                arbiter.next_lease()
            }
            Err(poisoned) => {
                let mut arbiter = poisoned.into_inner();
                arbiter.release(lease);
                arbiter.next_lease()
            }
        };
        let Some(next) = next else {
            return Ok(None);
        };
        if self.activate_promoted
            && let Err(error) = self
                .mediator
                .set_channel_grant(AudioChannel::Microphone, AudioGrant::On)
        {
            match self.microphone.lock() {
                Ok(mut arbiter) => arbiter.requeue_active(next),
                Err(poisoned) => poisoned.into_inner().requeue_active(next),
            }
            return Err(AudioControllerError::Mediator(error));
        }
        if self.activate_promoted {
            self.microphone_effect_applied = true;
        }
        Ok(Some(next))
    }
}

/// Validate an AudioService before controller registration.
pub fn register_service(service: &crate::AudioServiceSpec) -> Result<(), AudioControllerError> {
    validate_audio_service(service).map_err(|_| AudioControllerError::Admission)
}
