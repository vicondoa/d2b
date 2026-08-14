//! Typed AudioMediator service boundary.

use crate::{AudioGrant, LevelPercent};

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
    /// Apply a bounded level through the owner mediator.
    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError>;
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
}

impl AudioMediator for FakeAudioMediator {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        self.grant = grant;
        Ok(())
    }

    fn set_level(&mut self, level: LevelPercent) -> Result<(), AudioMediatorError> {
        if !self.owner {
            return Err(AudioMediatorError::ProjectionCannotOpenPipewire);
        }
        if self.host != HostAudioReadiness::Ready {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        self.level = Some(level);
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
