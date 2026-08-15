//! Typed AudioMediator service boundary.

use crate::{AudioGrant, LevelPercent};

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

/// Typed mediator failures.
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
}

impl AudioMediatorError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ProjectionCannotOpenPipewire => "audio-projection-pipewire-open-denied",
            Self::ProviderSessionUnavailable => "audio-provider-session-unavailable",
            Self::GuestSessionUnavailable => "audio-guest-session-unavailable",
            Self::LevelOutOfRange => "audio-level-out-of-range",
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
pub trait AudioMediator {
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

/// An owner or projection fake used by hermetic controller tests.
#[derive(Debug, Clone)]
pub struct FakeAudioMediator {
    owner: bool,
    host: HostAudioReadiness,
    guest: GuestAudioReadiness,
    grant: AudioGrant,
    level: Option<LevelPercent>,
    grant_calls: u32,
    level_calls: u32,
}

impl FakeAudioMediator {
    /// Construct an owner mediator whose host and guest paths are ready.
    pub fn ready() -> Self {
        Self {
            owner: true,
            host: HostAudioReadiness::Ready,
            guest: GuestAudioReadiness::Ready,
            grant: AudioGrant::Off,
            level: None,
            grant_calls: 0,
            level_calls: 0,
        }
    }

    /// Construct a projection that can only use an import stream.
    pub fn projection() -> Self {
        Self {
            owner: false,
            host: HostAudioReadiness::Unavailable,
            guest: GuestAudioReadiness::Ready,
            grant: AudioGrant::Off,
            level: None,
            grant_calls: 0,
            level_calls: 0,
        }
    }

    /// Construct a host-session failure.
    pub fn unavailable() -> Self {
        Self {
            owner: true,
            host: HostAudioReadiness::Unavailable,
            guest: GuestAudioReadiness::Ready,
            grant: AudioGrant::Off,
            level: None,
            grant_calls: 0,
            level_calls: 0,
        }
    }

    /// Return the last grant applied.
    pub const fn grant(&self) -> AudioGrant {
        self.grant
    }

    /// Return the last level applied.
    pub const fn level(&self) -> Option<LevelPercent> {
        self.level
    }

    /// Return the number of accepted grant operations.
    pub const fn grant_calls(&self) -> u32 {
        self.grant_calls
    }

    /// Return the number of accepted level operations.
    pub const fn level_calls(&self) -> u32 {
        self.level_calls
    }
}

impl AudioMediator for FakeAudioMediator {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        self.grant_calls = self.grant_calls.saturating_add(1);
        self.grant = grant;
        Ok(())
    }

    fn set_channel_grant(
        &mut self,
        _channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        self.set_grant(grant)
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        self.level_calls = self.level_calls.saturating_add(1);
        self.level = Some(level);
        Ok(())
    }

    fn set_channel_level(
        &mut self,
        _channel: AudioChannel,
        level: LevelPercent,
    ) -> Result<(), AudioMediatorError> {
        self.set_level(level)
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
