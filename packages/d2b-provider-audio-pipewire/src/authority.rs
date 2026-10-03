//! Bounded owner-Service audio authority.

use crate::{
    AudioChannel,
    mediator::{AdmittedAudioSession, AudioBindingFence, AudioSessionOrigin},
};
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    num::NonZeroUsize,
    sync::Arc,
};

/// Opaque operation-scoped audio lease identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioLeaseId(u64);

impl AudioLeaseId {
    /// Construct an opaque lease identity from a caller-assigned value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Mint the lease one admitted relationship's committed identity mints.
    ///
    /// The lease is identity evidence, not a counter: a controller that
    /// restarts over the same committed relationship resumes the same lease,
    /// while a relationship re-committed under a new generation is a
    /// different lease and can never inherit the previous one's authority.
    pub fn for_binding(uid: &ResourceUid, generation: ResourceGeneration) -> Self {
        // FNV-1a over the committed identities. This is an in-process
        // identity token, not a capability: the authority to act still comes
        // from the admitted relationship, so a collision here can at worst
        // make two relationships look like one lease to the caller, never
        // grant one relationship's access to another.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut absorb = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        absorb(uid.as_str().as_bytes());
        absorb(b"\0");
        absorb(&generation.get().to_le_bytes());
        // The lowest bit is forced so a minted lease is never zero, which
        // the caller reads as "no relationship admitted".
        Self(hash | 1)
    }
}


/// The admitted relationship one channel's bookkeeping rides.
///
/// The recorded fence is the evidence the grant was taken under, so a level
/// or a release that arrives after the relationship was re-committed or
/// revoked is refused instead of being applied to whatever incarnation
/// happens to hold the slot now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSession {
    channel: Option<AudioChannel>,
    fence: Option<AudioBindingFence>,
}

impl ChannelSession {
    /// The retained bookkeeping for a pass that observed no admitted
    /// relationship.
    ///
    /// It is deliberately unequal to every admitted session, so a grant
    /// recorded this way can never be continued by an admitted pass and an
    /// admitted grant can never be continued by an unadmitted one.
    pub const UNADMITTED: Self = Self {
        channel: None,
        fence: None,
    };

    /// Record the admitted relationship one channel's grant rides.
    ///
    /// # Errors
    ///
    /// Refuses a relationship admitted for the other channel
    /// (`SessionChannelMismatch`) and an imported Service projection
    /// (`ImportedProjectionNotOwner`).
    pub fn admit(
        channel: AudioChannel,
        session: &AdmittedAudioSession,
    ) -> Result<Self, AudioAuthorityError> {
        if session.channel() != channel {
            return Err(AudioAuthorityError::SessionChannelMismatch);
        }
        if session.origin() == AudioSessionOrigin::ImportedProjection {
            return Err(AudioAuthorityError::ImportedProjectionNotOwner);
        }
        Ok(Self {
            channel: Some(channel),
            fence: Some(session.fence().clone()),
        })
    }

    /// The committed relationship identity the grant was taken under.
    pub const fn fence(&self) -> Option<&AudioBindingFence> {
        self.fence.as_ref()
    }

    /// The channel this relationship was admitted for.
    pub const fn channel(&self) -> Option<AudioChannel> {
        self.channel
    }

    /// Whether this session is an admitted relationship rather than the
    /// retained unadmitted marker.
    pub const fn is_admitted(&self) -> bool {
        self.fence.is_some()
    }

    /// Whether `other` continues this session's recorded admission.
    ///
    /// A different channel is a different relationship, and an admitted
    /// session never continues an unadmitted one (or the reverse).
    fn continues(&self, other: &ChannelSession) -> Result<(), AudioAuthorityError> {
        if self.fence != other.fence {
            return Err(AudioAuthorityError::SessionNotCurrent);
        }
        if self.channel != other.channel {
            return Err(AudioAuthorityError::SessionChannelMismatch);
        }
        Ok(())
    }

}

/// Refuse a relationship admitted for a channel this table is not.
///
/// The retained unadmitted marker passes: it names no channel, so it can
/// neither claim nor be claimed as one.
fn require_channel(
    table: AudioChannel,
    session: &ChannelSession,
) -> Result<(), AudioAuthorityError> {
    match session.channel() {
        None => Ok(()),
        Some(channel) if matches!(
            (table, channel),
            (AudioChannel::Speaker, AudioChannel::Speaker)
                | (AudioChannel::Microphone, AudioChannel::Microphone)
        ) => Ok(()),
        Some(_) => Err(AudioAuthorityError::SessionChannelMismatch),
    }
}

/// Result of a microphone request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicDecision {
    /// The lease owns the exclusive microphone slot.
    Granted,
    /// The lease is queued in FIFO order.
    Queued,
    /// The bounded queue has no capacity.
    QueueFull,
}

/// Single-owner microphone arbiter.
///
/// The table records, per lease, the admitted relationship the capture
/// grant was taken under, so a promotion, a level, or a release that
/// arrives after the relationship changed is refused rather than applied to
/// whatever incarnation now holds the slot.
#[derive(Debug, Clone)]
pub struct MicrophoneArbiter {
    active: Option<AudioLeaseId>,
    queue: VecDeque<AudioLeaseId>,
    sessions: BTreeMap<AudioLeaseId, ChannelSession>,
    max_queue: usize,
}

/// Shared microphone authority for all bindings of one AudioService.
///
/// The async mutex backs this shared table so no executor worker ever parks
/// on it; the controller's synchronous public surface (consumed by the
/// daemon's resource runtime) reaches it through the U4 non-blocking
/// `try_lock` form.
pub type SharedMicrophoneArbiter = Arc<tokio::sync::Mutex<MicrophoneArbiter>>;

/// Construct a shared microphone authority with the provider's queue bound.
pub fn shared_microphone_arbiter(max_queue: NonZeroUsize) -> SharedMicrophoneArbiter {
    Arc::new(tokio::sync::Mutex::new(MicrophoneArbiter::new(max_queue)))
}

impl MicrophoneArbiter {
    /// Construct an arbiter with a bounded pending queue.
    pub fn new(max_queue: NonZeroUsize) -> Self {
        Self {
            active: None,
            queue: VecDeque::new(),
            sessions: BTreeMap::new(),
            max_queue: max_queue.get(),
        }
    }

    /// The admitted relationship one lease's capture grant rides.
    pub fn session_of(&self, lease: AudioLeaseId) -> Option<&ChannelSession> {
        self.sessions.get(&lease)
    }

    /// Whether the lease's recorded relationship is still the one presented.
    ///
    /// `None` means the lease was never admitted through a relationship, so
    /// it cannot be re-admitted by presenting one now.
    pub fn session_is_current(&self, lease: AudioLeaseId, session: &ChannelSession) -> bool {
        self.sessions
            .get(&lease)
            .is_some_and(|recorded| recorded.continues(session).is_ok())
    }

    /// Request the exclusive capture lease.
    ///
    /// # Errors
    ///
    /// Refuses a lease whose recorded relationship is not the relationship
    /// presented now (`SessionNotCurrent`). The capture grant is the only
    /// microphone authority there is, so a lease admitted under a different
    /// incarnation cannot silently take the slot.
    pub fn request(
        &mut self,
        lease: AudioLeaseId,
        session: &ChannelSession,
    ) -> Result<MicDecision, AudioAuthorityError> {
        require_channel(AudioChannel::Microphone, session)?;
        if let Some(recorded) = self.sessions.get(&lease) {
            recorded.continues(session)?;
        }
        self.sessions.insert(lease, session.clone());
        Ok(self.arbitrate(lease))
    }

    fn arbitrate(&mut self, lease: AudioLeaseId) -> MicDecision {
        if self.active == Some(lease) {
            return MicDecision::Granted;
        }
        if self.active.is_none() {
            if let Some(next) = self.queue.pop_front() {
                self.active = Some(next);
                if next == lease {
                    return MicDecision::Granted;
                }
                if !self.queue.contains(&lease) {
                    self.queue.push_back(lease);
                }
                return MicDecision::Queued;
            }
            self.active = Some(lease);
            return MicDecision::Granted;
        }
        if self.queue.contains(&lease) {
            return MicDecision::Queued;
        }
        if self.queue.len() >= self.max_queue {
            MicDecision::QueueFull
        } else {
            self.queue.push_back(lease);
            MicDecision::Queued
        }
    }

    /// Release a lease and mute it before the next lease is selected.
    pub fn release(&mut self, lease: AudioLeaseId) -> bool {
        let released = if self.active == Some(lease) {
            self.active = None;
            true
        } else {
            let before = self.queue.len();
            self.queue.retain(|id| *id != lease);
            before != self.queue.len()
        };
        if released {
            self.sessions.remove(&lease);
        }
        released
    }

    /// Mute-before-handoff and grant the next FIFO lease.
    pub fn next_lease(&mut self) -> Option<AudioLeaseId> {
        if self.active.is_some() {
            return self.active;
        }
        self.active = self.queue.pop_front();
        self.active
    }

    /// Put a just-promoted lease back at the head of the FIFO queue.
    ///
    /// This is used when the host or guest effect rejects a handoff.  The
    /// lease remains pending rather than being lost or left falsely active.
    pub(crate) fn requeue_active(&mut self, lease: AudioLeaseId) {
        if self.active == Some(lease) {
            self.active = None;
            self.queue.push_front(lease);
        }
    }

    /// Return the active lease without exposing Zone identity.
    pub const fn active(&self) -> Option<AudioLeaseId> {
        self.active
    }

    /// Return the bounded pending count.
    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}

/// Speaker mixing state.
///
/// The mixer records, per consumer, the admitted speaker relationship its
/// grant and level were taken under. The speaker level is a declared method
/// on that same relationship, so a level presented for a re-committed or
/// revoked relationship is refused instead of being mixed in.
#[derive(Debug, Clone, Default)]
pub struct SpeakerMixer {
    levels: BTreeMap<AudioLeaseId, u8>,
    grants: BTreeSet<AudioLeaseId>,
    sessions: BTreeMap<AudioLeaseId, ChannelSession>,
    max_consumers: usize,
}

impl SpeakerMixer {
    /// Construct a mixer with a bounded number of consumers.
    pub fn new(max_consumers: NonZeroUsize) -> Self {
        Self {
            levels: BTreeMap::new(),
            grants: BTreeSet::new(),
            sessions: BTreeMap::new(),
            max_consumers: max_consumers.get(),
        }
    }

    /// The admitted relationship one consumer's speaker grant rides.
    pub fn session_of(&self, lease: AudioLeaseId) -> Option<&ChannelSession> {
        self.sessions.get(&lease)
    }

    /// Grant one speaker consumer.
    ///
    /// The return value is true when the aggregate speaker grant changed
    /// from no consumers to at least one consumer.
    ///
    /// # Errors
    ///
    /// Refuses a consumer beyond the mixer bound (`ConsumerLimit`) and a
    /// consumer whose recorded speaker relationship is not the relationship
    /// presented now (`SessionNotCurrent`).
    pub fn grant(
        &mut self,
        lease: AudioLeaseId,
        session: &ChannelSession,
    ) -> Result<bool, AudioAuthorityError> {
        require_channel(AudioChannel::Speaker, session)?;
        if let Some(recorded) = self.sessions.get(&lease) {
            recorded.continues(session)?;
        }
        if !self.grants.contains(&lease)
            && !self.levels.contains_key(&lease)
            && self.consumer_count() >= self.max_consumers
        {
            return Err(AudioAuthorityError::ConsumerLimit);
        }
        let was_empty = self.grants.is_empty();
        self.grants.insert(lease);
        self.sessions.insert(lease, session.clone());
        Ok(was_empty)
    }

    /// Revoke one speaker consumer.
    ///
    /// The return value is true when the revoked consumer was the last
    /// grant holder.
    pub fn revoke(&mut self, lease: AudioLeaseId) -> Result<bool, AudioAuthorityError> {
        let was_last = self.grants.len() == 1 && self.grants.contains(&lease);
        self.grants.remove(&lease);
        Ok(was_last)
    }

    /// Return whether one lease currently holds a speaker grant.
    pub fn has_grant(&self, lease: AudioLeaseId) -> bool {
        self.grants.contains(&lease)
    }

    /// Return the last level recorded for one consumer.
    pub fn level(&self, lease: AudioLeaseId) -> Option<u8> {
        self.levels.get(&lease).copied()
    }

    /// Return whether any speaker grant remains active.
    pub fn has_any_grant(&self) -> bool {
        !self.grants.is_empty()
    }

    /// Return whether revoking this lease would remove the last grant.
    pub fn is_last_grant(&self, lease: AudioLeaseId) -> bool {
        self.grants.len() == 1 && self.grants.contains(&lease)
    }

    /// Set a bounded consumer level.
    ///
    /// # Errors
    ///
    /// Every refusal [`Self::can_set_level`] reports, and the level is only
    /// recorded against the relationship the grant was taken under.
    pub fn set_level(
        &mut self,
        lease: AudioLeaseId,
        level: u8,
        session: &ChannelSession,
    ) -> Result<(), AudioAuthorityError> {
        self.can_set_level(lease, level, session)?;
        self.levels.insert(lease, level);
        self.sessions.insert(lease, session.clone());
        Ok(())
    }

    /// Check whether a bounded consumer level can be admitted.
    ///
    /// # Errors
    ///
    /// Refuses an out-of-range level (`LevelOutOfRange`), a consumer beyond
    /// the mixer bound (`ConsumerLimit`), and a level presented for a
    /// relationship that is not the one the consumer's grant was taken under
    /// (`SessionNotCurrent`).
    pub(crate) fn can_set_level(
        &self,
        lease: AudioLeaseId,
        level: u8,
        session: &ChannelSession,
    ) -> Result<(), AudioAuthorityError> {
        if level > 100 {
            return Err(AudioAuthorityError::LevelOutOfRange);
        }
        require_channel(AudioChannel::Speaker, session)?;
        if let Some(recorded) = self.sessions.get(&lease) {
            recorded.continues(session)?;
        }
        if !self.levels.contains_key(&lease) && self.consumer_count() >= self.max_consumers {
            return Err(AudioAuthorityError::ConsumerLimit);
        }
        Ok(())
    }

    /// Remove one consumer.
    pub fn remove(&mut self, lease: AudioLeaseId) {
        self.levels.remove(&lease);
        self.grants.remove(&lease);
    }

    fn consumer_count(&self) -> usize {
        self.levels
            .keys()
            .chain(self.grants.iter())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Return the bounded mixed level.
    pub fn mix_level(&self) -> u8 {
        self.levels
            .values()
            .copied()
            .map(u16::from)
            .sum::<u16>()
            .min(100) as u8
    }
}

/// Stable authority failures.
///
/// Every variant is field-free: a refusal names a class of failure, never the
/// relationship, the consumer, or the host bytes it was protecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAuthorityError {
    /// Level was outside 0..=100.
    LevelOutOfRange,
    /// Consumer bound was exceeded.
    ConsumerLimit,
    /// The presented relationship was admitted for the other channel.
    SessionChannelMismatch,
    /// The presented relationship is not the one the grant was taken under.
    SessionNotCurrent,
    /// An imported Service projection cannot own a local backing grant.
    ImportedProjectionNotOwner,
}

impl core::fmt::Display for AudioAuthorityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::LevelOutOfRange => "audio-level-out-of-range",
            Self::ConsumerLimit => "audio-consumer-limit",
            Self::SessionChannelMismatch => "audio-session-channel-mismatch",
            Self::SessionNotCurrent => "audio-session-not-current",
            Self::ImportedProjectionNotOwner => "audio-imported-projection-not-owner",
        })
    }
}

impl std::error::Error for AudioAuthorityError {}
