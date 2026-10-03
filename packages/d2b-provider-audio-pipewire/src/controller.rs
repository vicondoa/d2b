//! AudioService and AudioBinding reconciliation through typed ports.

use crate::{
    AudioBindingSpec, AudioChannel, AudioGrant, AudioLeaseId, AudioMediator, AudioMediatorError,
    AudioReadiness, GuestAudioReadiness, HostAudioReadiness, MicDecision, SharedMicrophoneArbiter,
    SpeakerMixer, validate_audio_binding_in_zone,
    authority::{AudioAuthorityError, ChannelSession},
    mediator::{AdmittedAudioMediator, AudioSessionPlan},
};
use d2b_contracts_provider::v3::semantic_services::{
    SemanticFamily,
    child_resources::{
        ProcessChildKind, BindingChildPlacement, BindingChildRequest, BindingChildSet,
        explicit_binding_children,
    },
};
use d2b_contracts_resource::v3::{ExecutionDomain, ResourceRef};
use std::num::NonZeroUsize;
use tracing::{debug, warn};

const AUDIO_PROVIDER_REF: &str = "Provider/audio-pipewire";

/// Default shared-Runner repair interval for audio resources.
///
/// 300 seconds (5 minutes) bounds how long a failed audio worker can
/// stay unrepaired before the next resync re-runs the repair path,
/// while keeping the resync cadence well below the daemon's
/// operator-visible stall threshold.
pub const AUDIO_REPAIR_INTERVAL_SECS: u64 = 300;

/// The arbiter and mixer admission bound: how many pending microphone
/// leases or speaker consumers one controller admits before refusing
/// further admission.
pub const AUDIO_QUEUE_BOUND: NonZeroUsize = NonZeroUsize::new(64).expect("fixed nonzero bound");

const AUDIO_BINDING_CHILD_REQUESTS: [BindingChildRequest; 4] = [
    BindingChildRequest::process(
        ProcessChildKind::Process,
        BindingChildPlacement::Host,
        "host-effect",
        "Provider/system-minijail",
        "vhost-user-sound-worker",
        ExecutionDomain::System,
        "worker",
    ),
    BindingChildRequest::endpoint(BindingChildPlacement::Host, "host-endpoint", "host-effect"),
    BindingChildRequest::process(
        ProcessChildKind::Process,
        BindingChildPlacement::Guest,
        "guest-agent",
        "Provider/system-systemd",
        "guest-audio-agent",
        ExecutionDomain::System,
        "service",
    ),
    BindingChildRequest::endpoint(
        BindingChildPlacement::Guest,
        "guest-endpoint",
        "guest-agent",
    ),
];

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

/// Per-channel observed speaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSpeakerStatus {
    /// Last desired speaker grant.
    pub grant: AudioGrant,
    /// Last desired speaker level.
    pub level: Option<crate::LevelPercent>,
    /// Whether the speaker state is currently enforced.
    pub live_enforced: bool,
}

/// Per-channel observed microphone state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioMicrophoneStatus {
    /// Last desired microphone grant.
    pub grant: AudioGrant,
    /// Last desired microphone gain.
    pub gain: Option<crate::LevelPercent>,
    /// Whether the microphone state is currently enforced.
    pub live_enforced: bool,
    /// Current Service-level arbitration state.
    pub arbitration_state: AudioArbitrationState,
}

/// Closed microphone arbitration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioArbitrationState {
    /// This Binding does not request microphone capture.
    Inactive,
    /// This Binding is waiting for the Service microphone lease.
    Queued,
    /// This Binding owns the Service microphone lease.
    Active,
    /// The Service could not admit this Binding.
    Blocked,
}

/// The channels projected by an AudioBinding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioBindingChannels {
    /// Speaker observation.
    pub speaker: AudioSpeakerStatus,
    /// Microphone observation.
    pub mic: AudioMicrophoneStatus,
}

/// Aggregate host/guest enforcement posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEnforcementPosture {
    /// Both host and guest enforcement are available.
    HostAndGuest,
    /// Only host enforcement is available.
    HostOnly,
    /// Only guest enforcement is available.
    GuestOnly,
    /// No enforcement is currently available.
    None,
}

/// Where the most recent mutable audio setting was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioLastSetApplied {
    /// Applied to both host and guest.
    HostAndGuest,
    /// Applied to the host only.
    HostOnly,
    /// Applied to the guest only.
    GuestOnly,
    /// No setting was applied in the current reconcile.
    NotApplied,
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
    /// Last observed channel state.
    pub channels: AudioBindingChannels,
    /// Aggregate host/guest enforcement posture.
    pub enforcement_posture: AudioEnforcementPosture,
    /// Application path for the most recent setting.
    pub last_set_applied: AudioLastSetApplied,
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

impl std::error::Error for AudioControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission => None,
            Self::Mediator(error) => Some(error),
        }
    }
}

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

/// Reconcile output including the child resources owned by the Binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioReconcileResultWithChildren {
    /// Readiness and effect observations.
    pub result: AudioReconcileResult,
    /// UID-free Process and Endpoint intents.
    pub children: BindingChildSet,
}

/// Whether finalization enables the promoted microphone lease through this
/// binding's mediator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneHandoff {
    /// The promoted lease is enabled through this binding's mediator.
    Enable,
    /// The promoted lease is left inactive for the daemon to reconcile.
    Defer,
}

/// AudioBinding controller over existing audio policy and mediator ports.
#[derive(Debug)]
pub struct AudioBindingController<M: AudioMediator> {
    mediator: M,
    microphone: SharedMicrophoneArbiter,
    handoff: MicrophoneHandoff,
    microphone_effect_applied: bool,
    speaker: SpeakerMixer,
}

impl<M: AudioMediator> AudioBindingController<M> {
    /// Construct a controller with bounded arbitration state.
    ///
    /// Finalization enables the promoted microphone lease through this
    /// controller's mediator.
    pub fn new(mediator: M) -> Self {
        Self {
            mediator,
            microphone: crate::shared_microphone_arbiter(AUDIO_QUEUE_BOUND),
            handoff: MicrophoneHandoff::Enable,
            microphone_effect_applied: false,
            speaker: SpeakerMixer::new(AUDIO_QUEUE_BOUND),
        }
    }

    /// Construct a controller sharing one AudioService microphone authority.
    ///
    /// Finalization leaves the promoted lease inactive; the caller
    /// reconciles the owning binding so the effect is applied to the
    /// correct target.
    pub fn with_shared_microphone(mediator: M, microphone: SharedMicrophoneArbiter) -> Self {
        Self {
            mediator,
            microphone,
            handoff: MicrophoneHandoff::Defer,
            microphone_effect_applied: false,
            speaker: SpeakerMixer::new(AUDIO_QUEUE_BOUND),
        }
    }

    /// Borrow the mediator for status or test inspection.
    pub const fn mediator(&self) -> &M {
        &self.mediator
    }

    /// Build the explicit Host and Guest child resources for one Binding.
    ///
    /// The Binding and Service references are required inputs. A Ready
    /// Service cannot produce these children without an authored Binding.
    pub fn child_resources(
        binding_ref: &ResourceRef,
        binding: &AudioBindingSpec,
    ) -> Result<BindingChildSet, AudioControllerError> {
        crate::validate_audio_binding(binding).map_err(|error| {
            debug!(
                binding = %binding_ref.to_canonical_string(),
                error = %error,
                "audio binding admission rejected while synthesizing child resources"
            );
            AudioControllerError::Admission
        })?;
        explicit_binding_children(
            SemanticFamily::Audio,
            binding_ref.clone(),
            binding.service_ref.clone(),
            binding.target_ref.clone(),
            ResourceRef::parse(AUDIO_PROVIDER_REF).expect("audio Provider reference is canonical"),
            &AUDIO_BINDING_CHILD_REQUESTS,
        )
        .map_err(|error| {
            debug!(
                binding = %binding_ref.to_canonical_string(),
                error = %error,
                "audio binding child resource synthesis rejected"
            );
            AudioControllerError::Admission
        })
    }

    /// Reconcile a Binding and return the resource-backed child intents.
    pub fn reconcile_with_children(
        &mut self,
        binding_ref: &ResourceRef,
        binding: &AudioBindingSpec,
        service_zone: &str,
        lease: AudioLeaseId,
    ) -> Result<AudioReconcileResultWithChildren, AudioControllerError> {
        validate_audio_binding_in_zone(binding, service_zone).map_err(|error| {
            debug!(
                binding = %binding_ref.to_canonical_string(),
                zone = %service_zone,
                error = %error,
                "audio binding admission rejected during reconcile"
            );
            AudioControllerError::Admission
        })?;
        let children = Self::child_resources(binding_ref, binding)?;
        let result = self.reconcile(binding, service_zone, lease)?;
        Ok(AudioReconcileResultWithChildren { result, children })
    }

    /// Return the active microphone lease for status and recovery.
    ///
    /// Synchronous public surface (the daemon's resource runtime calls it
    /// without a runtime): the shared arbiter is reached through the U4
    /// non-blocking `try_lock` form. A collision - another binding's
    /// sub-microsecond bookkeeping on the same Service arbiter - reports no
    /// active lease and the caller re-checks on the next reconcile.
    pub fn active_microphone_lease(&self) -> Option<AudioLeaseId> {
        let Ok(arbiter) = self.microphone.try_lock() else {
            return None;
        };
        arbiter.active()
    }

    /// Reconcile one binding without opening a host handle itself.
    ///
    /// This is the retained pass for callers that have not yet observed an
    /// admitted endpoint relationship. It records the channel bookkeeping
    /// against an unadmitted session, and a later admitted pass for the same
    /// lease is refused rather than being allowed to inherit it.
    pub fn reconcile(
        &mut self,
        binding: &AudioBindingSpec,
        service_zone: &str,
        lease: AudioLeaseId,
    ) -> Result<AudioReconcileResult, AudioControllerError> {
        reconcile_channels(
            &mut self.mediator,
            &self.microphone,
            &mut self.microphone_effect_applied,
            &mut self.speaker,
            binding,
            service_zone,
            lease,
            self.handoff,
            ChannelSessions::UNADMITTED,
        )
    }

    /// Reconcile one binding behind the endpoint relationships the graph
    /// admitted for its channels.
    ///
    /// Every host and guest effect this pass reaches is carried by the exact
    /// relationship admitted for that channel, and the observed fences are
    /// what the carrier checks immediately before the effect runs, so a
    /// relationship that was revoked or re-committed stops serving effects
    /// even if the pass was already scheduled.
    pub fn reconcile_admitted(
        &mut self,
        binding: &AudioBindingSpec,
        service_zone: &str,
        lease: AudioLeaseId,
        sessions: &AudioSessionPlan,
        observed: &[crate::mediator::AudioBindingFence],
    ) -> Result<AudioReconcileResult, AudioControllerError> {
        let mut carrier = AdmittedAudioMediator::new(
            &mut self.mediator,
            sessions.clone(),
            observed.to_vec(),
        );
        reconcile_channels(
            &mut carrier,
            &self.microphone,
            &mut self.microphone_effect_applied,
            &mut self.speaker,
            binding,
            service_zone,
            lease,
            self.handoff,
            &ChannelSessions::from_plan(sessions),
        )
    }

    /// Revoke both channels after a restart that cannot adopt in-memory
    /// state.
    ///
    /// The microphone mute is reported before the speaker mute and each
    /// rides its own admitted relationship, so one channel being revoked
    /// never stands in for the other and a refusal on one leaves the other's
    /// revocation to the next pass.
    pub fn revoke_unmanaged_admitted(
        &mut self,
        sessions: &AudioSessionPlan,
        observed: &[crate::mediator::AudioBindingFence],
    ) -> Result<(), AudioControllerError> {
        let mut carrier = AdmittedAudioMediator::new(
            &mut self.mediator,
            sessions.clone(),
            observed.to_vec(),
        );
        revoke_unmanaged_on(&mut carrier)
    }

    /// Finalize one binding with mute-before-release ordering.
    ///
    /// The promoted microphone lease is enabled through this binding's
    /// mediator when the controller owns the microphone authority; shared
    /// controllers defer the activation to the daemon's reconcile.
    pub fn finalize(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        self.finalize_inner(lease)
    }

    /// Revoke effects after restart when no in-memory controller state can be
    /// adopted. The caller must first establish that no surviving Binding
    /// still owns the target's authority.
    pub fn revoke_unmanaged(&mut self) -> Result<(), AudioControllerError> {
        revoke_unmanaged_on(&mut self.mediator)
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
            warn!(
                lease = ?lease,
                error = %error,
                "promoted microphone activation mediation failed"
            );
            // U4 fail-closed: a busy arbiter skips the requeue; the next
            // reconcile re-arbitrates the still-pending lease.
            if let Ok(mut arbiter) = self.microphone.try_lock() {
                arbiter.requeue_active(lease);
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
                .map_err(|error| {
                    warn!(
                        lease = ?lease,
                        error = %error,
                        "speaker mute mediation failed during binding finalize"
                    );
                    AudioControllerError::Mediator(error)
                })?;
        }
        self.speaker.remove(lease);
        Ok(promoted)
    }

    fn release_microphone(
        &mut self,
        lease: AudioLeaseId,
    ) -> Result<Option<AudioLeaseId>, AudioControllerError> {
        release_microphone(
            &mut self.mediator,
            &self.microphone,
            &mut self.microphone_effect_applied,
            self.handoff,
            lease,
        )
    }
}

/// The admitted relationship each channel's bookkeeping rides in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelSessions {
    speaker: ChannelSession,
    microphone: ChannelSession,
}

impl ChannelSessions {
    /// The retained pass that has not observed an admitted relationship.
    ///
    /// A channel recorded against this session can never be continued by an
    /// admitted pass, because the recorded relationship and the admitted one
    /// are never equal.
    const UNADMITTED: &'static Self = &Self {
        speaker: ChannelSession::UNADMITTED,
        microphone: ChannelSession::UNADMITTED,
    };

    /// Project the admitted relationships of one plan onto the tables.
    ///
    /// A channel the plan does not carry falls back to the unadmitted
    /// session, which the carrier then refuses: an absent relationship is
    /// never treated as an admitted one.
    fn from_plan(sessions: &AudioSessionPlan) -> Self {
        let channel = |target: AudioChannel| -> ChannelSession {
            sessions
                .session(target)
                .and_then(|session| ChannelSession::admit(target, session).ok())
                .unwrap_or(ChannelSession::UNADMITTED)
        };
        Self {
            speaker: channel(AudioChannel::Speaker),
            microphone: channel(AudioChannel::Microphone),
        }
    }
}

/// Report one channel-authority refusal as an admission refusal.
///
/// The refusing stage is the relationship the grant or level was taken
/// under, and the typed reason travels in the log line; the returned class
/// stays the one the family's callers already classify as admission.
fn authority_refusal(
    zone: &str,
    stage: &'static str,
    error: AudioAuthorityError,
) -> AudioControllerError {
    debug!(
        zone = %zone,
        stage,
        error = %error,
        "audio channel authority refused the requested operation"
    );
    AudioControllerError::Admission
}

/// The active capture lease, without parking on the shared table.
fn active_microphone_lease(microphone: &SharedMicrophoneArbiter) -> Option<AudioLeaseId> {
    let Ok(arbiter) = microphone.try_lock() else {
        return None;
    };
    arbiter.active()
}

/// Revoke both channels through one mediator.
fn revoke_unmanaged_on<M: AudioMediator>(
    mediator: &mut M,
) -> Result<(), AudioControllerError> {
    mediator
        .set_channel_grant(AudioChannel::Microphone, AudioGrant::Off)
        .map_err(|error| {
            warn!(
                channel = "microphone",
                error = %error,
                "unmanaged microphone revoke mediation failed after restart"
            );
            AudioControllerError::Mediator(error)
        })?;
    mediator
        .set_channel_grant(AudioChannel::Speaker, AudioGrant::Off)
        .map_err(|error| {
            warn!(
                channel = "speaker",
                error = %error,
                "unmanaged speaker revoke mediation failed after restart"
            );
            AudioControllerError::Mediator(error)
        })?;
    Ok(())
}

/// Release the capture lease, muting before the handoff.
fn release_microphone<M: AudioMediator>(
    mediator: &mut M,
    microphone: &SharedMicrophoneArbiter,
    microphone_effect_applied: &mut bool,
    handoff: MicrophoneHandoff,
    lease: AudioLeaseId,
) -> Result<Option<AudioLeaseId>, AudioControllerError> {
    if active_microphone_lease(microphone) != Some(lease) {
        // U4 fail-closed: a busy arbiter defers the queue cleanup; the
        // next reconcile/finalize for this lease retries the release.
        if let Ok(mut arbiter) = microphone.try_lock() {
            arbiter.release(lease);
        }
        return Ok(None);
    }
    mediator
        .set_channel_grant(AudioChannel::Microphone, AudioGrant::Off)
        .map_err(|error| {
            warn!(
                lease = ?lease,
                error = %error,
                "microphone mute mediation failed during release"
            );
            AudioControllerError::Mediator(error)
        })?;
    *microphone_effect_applied = false;
    // U4 fail-closed: a busy arbiter defers the handoff; the caller's
    // next reconcile/finalize retries the release and promotion.
    let next = match microphone.try_lock() {
        Ok(mut arbiter) => {
            arbiter.release(lease);
            arbiter.next_lease()
        }
        Err(_) => None,
    };
    let Some(next) = next else {
        return Ok(None);
    };
    if handoff == MicrophoneHandoff::Enable
        && let Err(error) =
            mediator.set_channel_grant(AudioChannel::Microphone, AudioGrant::On)
    {
        warn!(
            lease = ?next,
            error = %error,
            "promoted microphone activation mediation failed during release"
        );
        // U4 fail-closed: a busy arbiter skips the requeue; the next
        // reconcile re-arbitrates the still-pending lease.
        if let Ok(mut arbiter) = microphone.try_lock() {
            arbiter.requeue_active(next);
        }
        return Err(AudioControllerError::Mediator(error));
    }
    if handoff == MicrophoneHandoff::Enable {
        *microphone_effect_applied = true;
    }
    Ok(Some(next))
}

/// Run one binding's channel bookkeeping behind one mediator and one
/// relationship per channel.
#[allow(
    clippy::too_many_arguments,
    reason = "the pass threads one mediator, two channel tables, and the admitted relationships"
)]
fn reconcile_channels<M: AudioMediator>(
    mediator: &mut M,
    microphone: &SharedMicrophoneArbiter,
    microphone_effect_applied: &mut bool,
    speaker: &mut SpeakerMixer,
    binding: &AudioBindingSpec,
    service_zone: &str,
    lease: AudioLeaseId,
    handoff: MicrophoneHandoff,
    sessions: &ChannelSessions,
) -> Result<AudioReconcileResult, AudioControllerError> {
    validate_audio_binding_in_zone(binding, service_zone).map_err(|error| {
        debug!(
            zone = %service_zone,
            error = %error,
            "audio binding admission rejected during reconcile"
        );
        AudioControllerError::Admission
    })?;
    let host_readiness = mediator.host_readiness();
    let guest_readiness = mediator.guest_readiness();
    let mut microphone_decision = None;
    let mut host_effect_applied = false;
    let mut guest_effect_applied = false;
    let mut speaker_live_enforced = false;
    let mut microphone_live_enforced = false;

    if binding.grants.mic == AudioGrant::On {
        let already_active = active_microphone_lease(microphone) == Some(lease);
        let session = &sessions.microphone;
        let decision = match microphone.try_lock() {
            Ok(mut arbiter) => arbiter
                .request(lease, session)
                .map_err(|error| authority_refusal(
                    service_zone,
                    "microphone arbitration rejected the presented relationship",
                    error,
                ))?,
            // U4 fail-closed: a busy shared arbiter refuses the request
            // rather than parking the caller's thread. The binding stays
            // muted and the next reconcile retries arbitration.
            Err(_) => MicDecision::QueueFull,
        };
        microphone_decision = Some(decision);
        match decision {
            MicDecision::Queued => debug!(
                zone = %service_zone,
                lease = ?lease,
                "microphone arbitration queued for binding"
            ),
            MicDecision::QueueFull => debug!(
                zone = %service_zone,
                lease = ?lease,
                "microphone arbitration queue full for binding"
            ),
            MicDecision::Granted => {}
        }
        let needs_effect =
            decision == MicDecision::Granted && (!already_active || !*microphone_effect_applied);
        if needs_effect {
            mediator
                .set_channel_grant(AudioChannel::Microphone, AudioGrant::On)
                .map_err(|error| {
                    warn!(
                        zone = %service_zone,
                        channel = "microphone",
                        lease = ?lease,
                        error = %error,
                        "microphone grant mediation failed for binding"
                    );
                    if !already_active {
                        // U4 fail-closed: a busy arbiter skips this
                        // rollback; the next reconcile re-arbitrates and
                        // the lease is never granted without an effect.
                        if let Ok(mut arbiter) = microphone.try_lock() {
                            arbiter.release(lease);
                        }
                    } else if let Ok(mut arbiter) = microphone.try_lock() {
                        arbiter.requeue_active(lease);
                    }
                    AudioControllerError::Mediator(error)
                })?;
            *microphone_effect_applied = true;
            host_effect_applied = true;
            guest_effect_applied = guest_readiness == GuestAudioReadiness::Ready;
            microphone_live_enforced = true;
        } else {
            microphone_live_enforced = *microphone_effect_applied;
        }
    } else {
        let was_active = active_microphone_lease(microphone) == Some(lease);
        release_microphone(
            mediator,
            microphone,
            microphone_effect_applied,
            handoff,
            lease,
        )?;
        if was_active {
            host_effect_applied = true;
            guest_effect_applied = guest_readiness == GuestAudioReadiness::Ready;
            microphone_live_enforced = true;
        }
    }
    if binding.grants.speaker == AudioGrant::On {
        let session = &sessions.speaker;
        let transition = speaker
            .grant(lease, session)
            .map_err(|error| authority_refusal(
                service_zone,
                "speaker grant bookkeeping rejected for binding",
                error,
            ))?;
        if transition {
            if let Err(error) =
                mediator.set_channel_grant(AudioChannel::Speaker, AudioGrant::On)
            {
                warn!(
                    zone = %service_zone,
                    channel = "speaker",
                    lease = ?lease,
                    error = %error,
                    "speaker grant mediation failed for binding"
                );
                if let Err(rollback_error) = speaker.revoke(lease) {
                    warn!(
                        zone = %service_zone,
                        lease = ?lease,
                        error = %rollback_error,
                        "speaker grant rollback failed after mediation failure"
                    );
                }
                return Err(AudioControllerError::Mediator(error));
            }
            host_effect_applied = true;
            guest_effect_applied |= guest_readiness == GuestAudioReadiness::Ready;
            speaker_live_enforced = true;
        } else {
            speaker_live_enforced = true;
        }
    } else if speaker.has_grant(lease) {
        let last = speaker.is_last_grant(lease);
        if last {
            mediator
                .set_channel_grant(AudioChannel::Speaker, AudioGrant::Off)
                .map_err(|error| {
                    warn!(
                        zone = %service_zone,
                        channel = "speaker",
                        lease = ?lease,
                        error = %error,
                        "speaker mute mediation failed for binding"
                    );
                    AudioControllerError::Mediator(error)
                })?;
        }
        speaker
            .revoke(lease)
            .map_err(|error| authority_refusal(
                service_zone,
                "speaker revoke bookkeeping rejected for binding",
                error,
            ))?;
        if last {
            host_effect_applied = true;
            guest_effect_applied |= guest_readiness == GuestAudioReadiness::Ready;
            speaker_live_enforced = true;
        }
    }
    if let Some(level) = binding.grants.speaker_level {
        let session = &sessions.speaker;
        speaker
            .can_set_level(lease, level.get(), session)
            .map_err(|error| authority_refusal(
                service_zone,
                "speaker level precondition rejected for binding",
                error,
            ))?;
        if speaker.level(lease) != Some(level.get()) {
            mediator
                .set_channel_level(AudioChannel::Speaker, level)
                .map_err(|error| {
                    warn!(
                        zone = %service_zone,
                        channel = "speaker",
                        lease = ?lease,
                        error = %error,
                        "speaker level mediation failed for binding"
                    );
                    AudioControllerError::Mediator(error)
                })?;
            host_effect_applied = true;
            guest_effect_applied |= guest_readiness == GuestAudioReadiness::Ready;
            speaker_live_enforced = true;
        }
        speaker
            .set_level(lease, level.get(), session)
            .map_err(|error| authority_refusal(
                service_zone,
                "speaker level bookkeeping rejected for binding",
                error,
            ))?;
        if speaker.level(lease) == Some(level.get()) {
            speaker_live_enforced = speaker_live_enforced || speaker.has_grant(lease);
        }
    }
    if let Some(gain) = binding.grants.mic_gain
        && microphone_decision == Some(MicDecision::Granted)
    {
        mediator
            .set_channel_level(AudioChannel::Microphone, gain)
            .map_err(|error| {
                warn!(
                    zone = %service_zone,
                    channel = "microphone",
                    lease = ?lease,
                    error = %error,
                    "microphone gain mediation failed for binding"
                );
                AudioControllerError::Mediator(error)
            })?;
        host_effect_applied = true;
        guest_effect_applied |= guest_readiness == GuestAudioReadiness::Ready;
        microphone_live_enforced = true;
    }

    let phase = match microphone_decision {
        Some(MicDecision::Queued) => AudioBindingPhase::Pending,
        Some(MicDecision::QueueFull) => AudioBindingPhase::Degraded,
        _ if mediator.readiness() == AudioReadiness::Ready => AudioBindingPhase::Ready,
        _ => {
            debug!(
                zone = %service_zone,
                lease = ?lease,
                "audio mediator readiness degraded for binding"
            );
            AudioBindingPhase::Degraded
        }
    };
    let arbitration_state = match microphone_decision {
        Some(MicDecision::Granted) => AudioArbitrationState::Active,
        Some(MicDecision::Queued) => AudioArbitrationState::Queued,
        Some(MicDecision::QueueFull) => AudioArbitrationState::Blocked,
        None => AudioArbitrationState::Inactive,
    };
    let enforcement_posture = match (
        host_effect_applied || speaker_live_enforced || microphone_live_enforced,
        guest_effect_applied,
    ) {
        (true, true) => AudioEnforcementPosture::HostAndGuest,
        (true, false) => AudioEnforcementPosture::HostOnly,
        (false, true) => AudioEnforcementPosture::GuestOnly,
        (false, false) => AudioEnforcementPosture::None,
    };
    let last_set_applied = match (host_effect_applied, guest_effect_applied) {
        (true, true) => AudioLastSetApplied::HostAndGuest,
        (true, false) => AudioLastSetApplied::HostOnly,
        (false, true) => AudioLastSetApplied::GuestOnly,
        (false, false) => AudioLastSetApplied::NotApplied,
    };
    Ok(AudioReconcileResult {
        status: AudioBindingStatus {
            phase,
            host_readiness,
            guest_readiness,
            microphone: microphone_decision,
            channels: AudioBindingChannels {
                speaker: AudioSpeakerStatus {
                    grant: binding.grants.speaker,
                    level: binding.grants.speaker_level,
                    live_enforced: speaker_live_enforced,
                },
                mic: AudioMicrophoneStatus {
                    grant: binding.grants.mic,
                    gain: binding.grants.mic_gain,
                    live_enforced: microphone_live_enforced,
                    arbitration_state,
                },
            },
            enforcement_posture,
            last_set_applied,
        },
        host_effect_applied,
        guest_effect_applied,
    })
}
