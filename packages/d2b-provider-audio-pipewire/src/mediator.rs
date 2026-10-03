//! Typed AudioMediator service boundary.

use crate::{AudioGrant, LevelPercent};
use d2b_contracts_resource::v3::{
    EndpointAttachmentKind, EndpointBindingRequest, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneRevision,
};

/// Audio stream direction for broker and guest-agent effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannel {
    /// Guest-to-host capture stream.
    Microphone,
    /// Host-to-guest playback stream.
    Speaker,
}

/// Host-side mediator readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAudioReadiness {
    /// The user-session PipeWire portal is usable.
    Ready,
    /// The host portal is unavailable.
    Unavailable,
}

/// Guest-side audio agent readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAudioReadiness {
    /// Guest frontend and agent are usable.
    Ready,
    /// Guest frontend or agent is unavailable.
    Unavailable,
}

/// Combined readiness retained for status projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioReadiness {
    /// Both sides are ready for an owner binding.
    Ready,
    /// At least one side is unavailable.
    Unavailable,
}

/// Every audio stream direction, in declaration order.
pub const AUDIO_CHANNELS: [AudioChannel; 2] = [AudioChannel::Speaker, AudioChannel::Microphone];

/// The declared purpose of the speaker channel's endpoint relationship.
pub const AUDIO_SPEAKER_PURPOSE: &str = "audio-speaker-session";

/// The declared purpose of the microphone channel's endpoint relationship.
pub const AUDIO_MICROPHONE_PURPOSE: &str = "audio-microphone-session";

/// The provider operation the speaker channel's Service declares.
pub const AUDIO_PLAYBACK_OPERATION: &str = "playback";

/// The provider operation the microphone channel's Service declares.
pub const AUDIO_CAPTURE_OPERATION: &str = "capture";

impl AudioChannel {
    /// The bounded usage purpose one channel's endpoint relationship declares.
    ///
    /// The purpose travels inside the admitted `EndpointBinding` request, so
    /// a channel is only ever reached through a relationship the graph
    /// classified onto exactly that channel. Restating the microphone's
    /// relationship as a speaker one does not produce a speaker grant: the
    /// two purposes are separate relationships with separate lifecycle.
    pub const fn declared_purpose(self) -> &'static str {
        match self {
            Self::Speaker => AUDIO_SPEAKER_PURPOSE,
            Self::Microphone => AUDIO_MICROPHONE_PURPOSE,
        }
    }

    /// The stable consumer slot one channel's relationship uses.
    ///
    /// Speaker and microphone are separate slots on the same consumer, so
    /// revoking one leaves the other relationship intact.
    pub const fn binding_slot(self) -> &'static str {
        match self {
            Self::Speaker => "speaker",
            Self::Microphone => "microphone",
        }
    }

    /// The provider operation one channel's Service declares.
    pub const fn declared_operation(self) -> &'static str {
        match self {
            Self::Speaker => AUDIO_PLAYBACK_OPERATION,
            Self::Microphone => AUDIO_CAPTURE_OPERATION,
        }
    }

    /// The PipeWire media class one channel's stream is selected by.
    pub const fn media_class(self) -> &'static str {
        match self {
            Self::Speaker => "Stream/Output/Audio",
            Self::Microphone => "Stream/Input/Audio",
        }
    }
}

/// Typed mediator failures.
///
/// Every variant is a closed, field-free refusal. A variant that means "the
/// exact endpoint relationship this effect rides is not admitted" is
/// distinct from one that means "the admitted relationship is no longer
/// current", because the first is a composition error the caller fixes by
/// requesting a relationship and the second is a lifecycle transition the
/// caller re-observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMediatorError {
    /// A projection cannot open the owner PipeWire session.
    ProjectionCannotOpenPipewire,
    /// The user-session portal is unavailable.
    ProviderSessionUnavailable,
    /// The guest agent is unavailable.
    GuestSessionUnavailable,
    /// A level was outside the closed range.
    LevelOutOfRange,
    /// The channel's exact `EndpointBinding` is not admitted.
    EndpointBindingNotAdmitted,
    /// The admitted relationship's fence is no longer current: the binding
    /// was re-committed, revoked, or re-admitted under a new generation.
    EndpointBindingNotCurrent,
    /// The request names a different channel's declared purpose, so it is a
    /// different relationship rather than this channel's.
    EndpointBindingPurposeMismatch,
    /// The request asks for an attachment form an audio consumer cannot use.
    EndpointAttachmentUnsupported,
    /// An imported Service projection cannot mint a local backing grant.
    ImportedProjectionCannotGrant,
}

impl AudioMediatorError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ProjectionCannotOpenPipewire => "audio-projection-pipewire-open-denied",
            Self::ProviderSessionUnavailable => "audio-provider-session-unavailable",
            Self::GuestSessionUnavailable => "audio-guest-session-unavailable",
            Self::LevelOutOfRange => "audio-level-out-of-range",
            Self::EndpointBindingNotAdmitted => "audio-endpoint-binding-not-admitted",
            Self::EndpointBindingNotCurrent => "audio-endpoint-binding-not-current",
            Self::EndpointBindingPurposeMismatch => "audio-endpoint-binding-purpose-mismatch",
            Self::EndpointAttachmentUnsupported => "audio-endpoint-attachment-unsupported",
            Self::ImportedProjectionCannotGrant => "audio-imported-projection-cannot-grant",
        }
    }
}

impl core::fmt::Display for AudioMediatorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AudioMediatorError {}

/// Effect-port contract used by the AudioBinding controller.
///
/// The supertraits keep the trait object usable across the family's
/// async service boundaries: the effects service holds the mediator behind
/// a trait object and its futures must be `Send`, so a boxed mediator must
/// be `Send + Sync` with the concrete mediators.
pub trait AudioMediator: Send + Sync {
    /// Apply an on/off grant through the owner mediator.
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError>;
    /// Apply an on/off grant to one stream direction.
    ///
    /// The default preserves compatibility with older mediators that expose
    /// one aggregate grant while production mediators can keep microphone and
    /// speaker state independent.
    fn set_channel_grant(
        &mut self,
        _channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        self.set_grant(grant)
    }
    /// Apply a bounded level through the owner mediator.
    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError>;
    /// Apply a bounded level to one stream direction.
    fn set_channel_level(
        &mut self,
        _channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        self.set_level(level)
    }
    /// Return combined readiness.
    fn readiness(&self) -> AudioReadiness;
    /// Return host readiness separately from guest readiness.
    fn host_readiness(&self) -> HostAudioReadiness;
    /// Return guest readiness separately from host readiness.
    fn guest_readiness(&self) -> GuestAudioReadiness;
}

/// What one declared audio method changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEffectKind {
    /// The on/off channel grant.
    Grant,
    /// The bounded channel level or gain.
    Level,
}

/// One declared method the audio family serves.
///
/// A channel effect is a named method on the audio family's Service rather
/// than a broker-specific action variant: the method name is what the
/// declared contract carries, and the channel it names is what keeps speaker
/// and microphone separate admissions rather than one shared grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDeclaredMethod {
    channel: AudioChannel,
    effect: AudioEffectKind,
    name: &'static str,
}

impl AudioDeclaredMethod {
    /// The declared speaker-grant method.
    pub const fn speaker_grant() -> Self {
        Self {
            channel: AudioChannel::Speaker,
            effect: AudioEffectKind::Grant,
            name: "set-speaker-grant",
        }
    }

    /// The declared speaker-level method.
    pub const fn speaker_level() -> Self {
        Self {
            channel: AudioChannel::Speaker,
            effect: AudioEffectKind::Level,
            name: "set-speaker-level",
        }
    }

    /// The declared microphone-grant method.
    pub const fn microphone_grant() -> Self {
        Self {
            channel: AudioChannel::Microphone,
            effect: AudioEffectKind::Grant,
            name: "set-microphone-grant",
        }
    }

    /// The declared microphone-gain method.
    pub const fn microphone_level() -> Self {
        Self {
            channel: AudioChannel::Microphone,
            effect: AudioEffectKind::Level,
            name: "set-microphone-gain",
        }
    }

    /// The channel this method acts on.
    pub const fn channel(self) -> AudioChannel {
        self.channel
    }

    /// What this method changes.
    pub const fn effect(self) -> AudioEffectKind {
        self.effect
    }

    /// The declared method name.
    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// The closed declared method vocabulary of the audio family.
///
/// One entry per channel effect, so there is exactly one declared source for
/// every audio operation and a caller cannot name an undeclared effect.
pub const AUDIO_DECLARED_METHODS: [AudioDeclaredMethod; 4] = [
    AudioDeclaredMethod::speaker_grant(),
    AudioDeclaredMethod::speaker_level(),
    AudioDeclaredMethod::microphone_grant(),
    AudioDeclaredMethod::microphone_level(),
];

/// The declared method one channel effect names, or `None` when the family
/// declares no such method.
pub fn audio_declared_method(
    channel: AudioChannel,
    effect: AudioEffectKind,
) -> Option<AudioDeclaredMethod> {
    AUDIO_DECLARED_METHODS
        .into_iter()
        .find(|method| method.channel == channel && method.effect == effect)
}

/// The committed identity one admitted relationship carries.
///
/// The fence is what an effect is re-checked against immediately before it
/// runs, so a relationship that was re-committed, revoked, or re-admitted
/// under a new generation stops serving effects without the caller having to
/// observe the change first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBindingFence {
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
}

impl AudioBindingFence {
    /// Construct one fence from a relationship's committed identities.
    pub const fn new(
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> Self {
        Self {
            uid,
            generation,
            revision,
        }
    }

    /// The relationship's committed uid.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// The committed row generation the admission was fenced against.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// The Zone revision the admission was fenced against.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Whether `observed` still describes this exact relationship.
    ///
    /// A fence from an older generation or an older Zone revision is not
    /// current: it describes an incarnation that no longer exists, so an
    /// effect carried by it must not run.
    pub fn matches(&self, observed: &AudioBindingFence) -> bool {
        self.uid == observed.uid
            && self.generation == observed.generation
            && self.revision == observed.revision
    }
}

/// Where the Service this relationship serves its channel from came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSessionOrigin {
    /// A locally owned AudioService with its own backing Endpoint.
    Owner,
    /// A ResourceImport-backed projection of another Zone's AudioService.
    ImportedProjection,
}

/// One admitted exact-endpoint relationship serving one audio channel.
///
/// This is the only thing an audio host or guest effect may be carried by.
/// It carries the committed request, the committed relationship identity,
/// and the origin of the Service it serves; it deliberately carries no socket
/// path, no runtime directory, no tool path, and no environment variable, so
/// no ambient runtime value can redirect a channel to a neighbouring
/// session.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedAudioSession {
    channel: AudioChannel,
    request: EndpointBindingRequest,
    fence: AudioBindingFence,
    origin: AudioSessionOrigin,
}

impl core::fmt::Debug for AdmittedAudioSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedAudioSession")
            .field("channel", &self.channel)
            .field("origin", &self.origin)
            .field("attachment", &self.request.attachment())
            .field("slot", &self.request.slot().as_str())
            .field("fence", &self.fence)
            .finish_non_exhaustive()
    }
}

impl AdmittedAudioSession {
    /// Admit one committed endpoint request for one channel.
    ///
    /// # Errors
    ///
    /// Refuses a request whose declared purpose belongs to another channel
    /// (`EndpointBindingPurposeMismatch`) and an attachment form an audio
    /// consumer cannot use (`EndpointAttachmentUnsupported`). Both refusals
    /// happen before the relationship can carry any effect.
    pub fn new(
        channel: AudioChannel,
        request: EndpointBindingRequest,
        fence: AudioBindingFence,
        origin: AudioSessionOrigin,
    ) -> Result<Self, AudioMediatorError> {
        if request.attachment() != EndpointAttachmentKind::Connect {
            return Err(AudioMediatorError::EndpointAttachmentUnsupported);
        }
        if request.purpose().as_str() != channel.declared_purpose() {
            return Err(AudioMediatorError::EndpointBindingPurposeMismatch);
        }
        Ok(Self {
            channel,
            request,
            fence,
            origin,
        })
    }

    /// The channel this relationship serves.
    pub const fn channel(&self) -> AudioChannel {
        self.channel
    }

    /// The exact committed endpoint request.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }

    /// The exact `Endpoint` this relationship's source names.
    pub const fn source_ref(&self) -> &ResourceRef {
        self.request.source_ref()
    }

    /// The admitted consumer this relationship delivers to.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        self.request.consumer_ref()
    }

    /// The committed relationship identity.
    pub const fn fence(&self) -> &AudioBindingFence {
        &self.fence
    }

    /// Whether this relationship came from an imported Service projection.
    pub const fn origin(&self) -> AudioSessionOrigin {
        self.origin
    }

    /// Whether this relationship still carries the committed identity it was
    /// admitted against.
    pub fn is_current(&self, observed: &AudioBindingFence) -> bool {
        self.fence.matches(observed)
    }

    /// Whether one declared method is this relationship's to serve.
    ///
    /// A method naming the other channel never is: that is what keeps a
    /// microphone admission from producing a speaker grant and the reverse.
    pub fn serves(&self, method: AudioDeclaredMethod) -> bool {
        matches!(
            (method.channel, self.channel),
            (AudioChannel::Speaker, AudioChannel::Speaker)
                | (AudioChannel::Microphone, AudioChannel::Microphone)
        )
    }

    /// Admit one effect on this relationship.
    ///
    /// # Errors
    ///
    /// Returns the refusal for a method on another channel
    /// (`EndpointBindingPurposeMismatch`), a fence that is no longer current
    /// (`EndpointBindingNotCurrent`), and an imported projection asking for
    /// a local backing grant (`ImportedProjectionCannotGrant`).
    pub fn admit_effect(
        &self,
        method: AudioDeclaredMethod,
        observed: &AudioBindingFence,
    ) -> Result<(), AudioMediatorError> {
        if !self.serves(method) {
            return Err(AudioMediatorError::EndpointBindingPurposeMismatch);
        }
        if self.origin == AudioSessionOrigin::ImportedProjection {
            return Err(AudioMediatorError::ImportedProjectionCannotGrant);
        }
        if !self.is_current(observed) {
            return Err(AudioMediatorError::EndpointBindingNotCurrent);
        }
        Ok(())
    }
}

/// One admitted relationship per channel, as a reconcile pass sees them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioSessionPlan {
    speaker: Option<AdmittedAudioSession>,
    microphone: Option<AdmittedAudioSession>,
}

impl AudioSessionPlan {
    /// Build one plan from the admitted relationships it holds.
    pub const fn new(
        speaker: Option<AdmittedAudioSession>,
        microphone: Option<AdmittedAudioSession>,
    ) -> Self {
        Self {
            speaker,
            microphone,
        }
    }

    /// The relationship admitted for one channel, when there is one.
    pub const fn session(&self, channel: AudioChannel) -> Option<&AdmittedAudioSession> {
        match channel {
            AudioChannel::Speaker => self.speaker.as_ref(),
            AudioChannel::Microphone => self.microphone.as_ref(),
        }
    }

    /// Every admitted relationship, in channel declaration order.
    pub fn admitted(&self) -> impl Iterator<Item = &AdmittedAudioSession> {
        AUDIO_CHANNELS
            .into_iter()
            .filter_map(|channel| self.session(channel))
    }

    /// Whether every admitted relationship is still current against
    /// `observed`.
    ///
    /// A plan with no admitted relationship is not ready: "nothing admitted"
    /// is an absence of evidence, never evidence of readiness.
    pub fn all_current(&self, observed: &[AudioBindingFence]) -> bool {
        let mut saw_one = false;
        for session in self.admitted() {
            saw_one = true;
            if !observed.iter().any(|fence| session.is_current(fence)) {
                return false;
            }
        }
        saw_one
    }
}

/// The mediator every audio effect is carried by once the graph admitted the
/// channel's exact endpoint relationship.
///
/// The wrapper adds no behaviour of its own beyond admission: it resolves
/// the relationship one channel's declared method rides, refuses an effect
/// whose fence is no longer current, and reports host readiness from the
/// admitted host relationships while guest readiness keeps coming from the
/// Guest's own agent. A host-side revocation therefore changes the host
/// observation without inventing a Guest one.
#[derive(Debug)]
pub struct AdmittedAudioMediator<M> {
    inner: M,
    plan: AudioSessionPlan,
    observed: Vec<AudioBindingFence>,
}

impl<M: AudioMediator> AdmittedAudioMediator<M> {
    /// Wrap one mediator behind the admitted relationships of one plan.
    pub fn new(inner: M, plan: AudioSessionPlan, observed: Vec<AudioBindingFence>) -> Self {
        Self {
            inner,
            plan,
            observed,
        }
    }

    /// Borrow the admitted relationship set.
    pub const fn plan(&self) -> &AudioSessionPlan {
        &self.plan
    }

    /// Borrow the mediator the admitted effects are carried to.
    pub const fn inner(&self) -> &M {
        &self.inner
    }

    fn admit(&self, method: AudioDeclaredMethod) -> Result<(), AudioMediatorError> {
        let session = self
            .plan
            .session(method.channel())
            .ok_or(AudioMediatorError::EndpointBindingNotAdmitted)?;
        let observed = self
            .observed
            .iter()
            .find(|fence| session.is_current(fence))
            .ok_or(AudioMediatorError::EndpointBindingNotCurrent)?;
        session.admit_effect(method, observed)
    }
}

impl<M: AudioMediator> AudioMediator for AdmittedAudioMediator<M> {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        self.set_channel_grant(AudioChannel::Speaker, grant)
    }

    fn set_channel_grant(
        &mut self,
        channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        let method = audio_declared_method(channel, AudioEffectKind::Grant)
            .ok_or(AudioMediatorError::EndpointBindingNotAdmitted)?;
        self.admit(method)?;
        self.inner.set_channel_grant(channel, grant)
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        self.set_channel_level(AudioChannel::Speaker, level)
    }

    fn set_channel_level(
        &mut self,
        channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        let method = audio_declared_method(channel, AudioEffectKind::Level)
            .ok_or(AudioMediatorError::EndpointBindingNotAdmitted)?;
        self.admit(method)?;
        self.inner.set_channel_level(channel, level)
    }

    fn readiness(&self) -> AudioReadiness {
        match (self.host_readiness(), self.guest_readiness()) {
            (HostAudioReadiness::Ready, GuestAudioReadiness::Ready) => AudioReadiness::Ready,
            _ => AudioReadiness::Unavailable,
        }
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        if self.plan.all_current(&self.observed) {
            HostAudioReadiness::Ready
        } else {
            HostAudioReadiness::Unavailable
        }
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        self.inner.guest_readiness()
    }
}

/// An owner or projection fake used by hermetic controller tests.
///
/// The fake observes each stream direction separately, because speaker and
/// microphone admission are separate relationships: a fake that collapsed
/// both into one observation could not tell a per-channel refusal from a
/// per-channel success.
#[derive(Debug, Clone)]
pub struct FakeAudioMediator {
    owner: bool,
    host: HostAudioReadiness,
    guest: GuestAudioReadiness,
    grants: [AudioGrant; 2],
    levels: [Option<LevelPercent>; 2],
    grant_calls: [u32; 2],
    level_calls: [u32; 2],
}

impl FakeAudioMediator {
    const fn new(owner: bool, host: HostAudioReadiness) -> Self {
        Self {
            owner,
            host,
            guest: GuestAudioReadiness::Ready,
            grants: [AudioGrant::Off; 2],
            levels: [None; 2],
            grant_calls: [0; 2],
            level_calls: [0; 2],
        }
    }

    /// Construct an owner mediator whose host and guest paths are ready.
    pub const fn ready() -> Self {
        Self::new(true, HostAudioReadiness::Ready)
    }

    /// Construct a projection that can only use an import stream.
    pub const fn projection() -> Self {
        Self::new(false, HostAudioReadiness::Unavailable)
    }

    /// Construct a host-session failure.
    pub const fn unavailable() -> Self {
        Self::new(true, HostAudioReadiness::Unavailable)
    }

    const fn slot(channel: AudioChannel) -> usize {
        match channel {
            AudioChannel::Speaker => 0,
            AudioChannel::Microphone => 1,
        }
    }

    /// Return the last grant applied to the speaker channel.
    pub const fn grant(&self) -> AudioGrant {
        self.grants[0]
    }

    /// Return the last grant applied to the microphone channel.
    pub const fn microphone_grant(&self) -> AudioGrant {
        self.grants[1]
    }

    /// Return the last grant applied to one channel.
    pub const fn channel_grant(&self, channel: AudioChannel) -> AudioGrant {
        self.grants[Self::slot(channel)]
    }

    /// Return the last speaker level applied.
    pub const fn level(&self) -> Option<LevelPercent> {
        self.levels[0]
    }

    /// Return the last microphone gain applied.
    pub const fn microphone_level(&self) -> Option<LevelPercent> {
        self.levels[1]
    }

    /// Return the last level applied to one channel.
    pub const fn channel_level(&self, channel: AudioChannel) -> Option<LevelPercent> {
        self.levels[Self::slot(channel)]
    }

    /// Return the number of accepted speaker grant operations.
    pub const fn grant_calls(&self) -> u32 {
        self.grant_calls[0]
    }

    /// Return the number of accepted speaker level operations.
    pub const fn level_calls(&self) -> u32 {
        self.level_calls[0]
    }

    /// Return the number of accepted grant operations on one channel.
    pub const fn channel_grant_calls(&self, channel: AudioChannel) -> u32 {
        self.grant_calls[Self::slot(channel)]
    }

    /// Return the number of accepted level operations on one channel.
    pub const fn channel_level_calls(&self, channel: AudioChannel) -> u32 {
        self.level_calls[Self::slot(channel)]
    }
}

impl AudioMediator for FakeAudioMediator {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        self.set_channel_grant(AudioChannel::Speaker, grant)
    }

    fn set_channel_grant(
        &mut self,
        channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        let slot = Self::slot(channel);
        self.grant_calls[slot] = self.grant_calls[slot].saturating_add(1);
        self.grants[slot] = grant;
        Ok(())
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        self.set_channel_level(AudioChannel::Speaker, level)
    }

    fn set_channel_level(
        &mut self,
        channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        let slot = Self::slot(channel);
        self.level_calls[slot] = self.level_calls[slot].saturating_add(1);
        self.levels[slot] = Some(level);
        Ok(())
    }

    fn readiness(&self) -> AudioReadiness {
        if self.host == HostAudioReadiness::Ready && self.guest == GuestAudioReadiness::Ready {
            AudioReadiness::Ready
        } else {
            AudioReadiness::Unavailable
        }
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        self.host
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        self.guest
    }
}

/// A boxed mediator is a mediator: the family's facet boundary hands the
/// controller a `Box<dyn AudioMediator>` (the daemon's broker-backed
/// mediator behind the declared facet), so the controller's `M: AudioMediator`
/// bound is satisfied by the trait object the owner crate builds.
impl AudioMediator for Box<dyn AudioMediator + '_> {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        self.as_mut().set_grant(grant)
    }

    fn set_channel_grant(
        &mut self,
        channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        self.as_mut().set_channel_grant(channel, grant)
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        self.as_mut().set_level(level)
    }

    fn set_channel_level(
        &mut self,
        channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        self.as_mut().set_channel_level(channel, level)
    }

    fn readiness(&self) -> AudioReadiness {
        self.as_ref().readiness()
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        self.as_ref().host_readiness()
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        self.as_ref().guest_readiness()
    }
}

/// A mutable borrow of a mediator is a mediator, so the controller can run
/// one pass behind the admission gate without moving its own mediator out of
/// the controller.
impl<M: AudioMediator + ?Sized> AudioMediator for &mut M {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        (**self).set_grant(grant)
    }

    fn set_channel_grant(
        &mut self,
        channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        (**self).set_channel_grant(channel, grant)
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        (**self).set_level(level)
    }

    fn set_channel_level(
        &mut self,
        channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        (**self).set_channel_level(channel, level)
    }

    fn readiness(&self) -> AudioReadiness {
        (**self).readiness()
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        (**self).host_readiness()
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        (**self).guest_readiness()
    }
}

/// Whether one committed `AudioService` row declares one provider operation.
pub fn service_declares_operation(
    spec: &crate::resource_type::AudioServiceSpec,
    operation: &str,
) -> bool {
    spec.operations.iter().any(|declared| declared == operation)
}

/// Whether one committed `AudioService` row declares the operation behind
/// one channel.
///
/// A binding may only request a relationship for a channel its Service
/// actually declared, so declaring one operation cannot reach the other
/// channel's methods.
pub fn service_declares_channel(
    spec: &crate::resource_type::AudioServiceSpec,
    channel: AudioChannel,
) -> bool {
    service_declares_operation(spec, channel.declared_operation())
}

/// The backing `Endpoint` an owner `AudioService` commits for its own
/// effects.
///
/// The endpoint's locator stays with the Endpoint owner: the Service row
/// names the exact row, and every effect it enables is admitted as an
/// `EndpointBinding` on that row rather than as access to whatever directory
/// happens to hold its socket. A ResourceImport projection declares no local
/// backing endpoint at all, so it has nothing for a local binding to request.
pub fn service_backing_endpoint(
    spec: &crate::resource_type::AudioServiceSpec,
) -> Option<&ResourceRef> {
    if spec.service_role != crate::resource_type::AudioServiceRole::Owner {
        return None;
    }
    spec.implementation_endpoint_refs.first()
}
