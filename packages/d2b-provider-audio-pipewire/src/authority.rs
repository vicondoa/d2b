//! Bounded owner-Service audio authority.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

/// Opaque operation-scoped audio lease identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioLeaseId(u64);

impl AudioLeaseId {
    /// Construct an opaque lease identity from a caller-assigned value.
    pub const fn new(value: u64) -> Self {
        Self(value)
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
#[derive(Debug, Clone)]
pub struct MicrophoneArbiter {
    active: Option<AudioLeaseId>,
    queue: VecDeque<AudioLeaseId>,
    max_queue: usize,
}

/// Shared microphone authority for all bindings of one AudioService.
pub type SharedMicrophoneArbiter = Arc<Mutex<MicrophoneArbiter>>;

/// Construct a shared microphone authority with the provider's queue bound.
pub fn shared_microphone_arbiter(max_queue: usize) -> SharedMicrophoneArbiter {
    Arc::new(Mutex::new(MicrophoneArbiter::new(max_queue)))
}

impl MicrophoneArbiter {
    /// Construct an arbiter with a bounded pending queue.
    pub fn new(max_queue: usize) -> Self {
        assert!(max_queue > 0);
        Self {
            active: None,
            queue: VecDeque::new(),
            max_queue,
        }
    }

    /// Request the exclusive capture lease.
    pub fn request(&mut self, lease: AudioLeaseId) -> MicDecision {
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
        if self.active == Some(lease) {
            self.active = None;
            return true;
        }
        let before = self.queue.len();
        self.queue.retain(|id| *id != lease);
        before != self.queue.len()
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
#[derive(Debug, Clone, Default)]
pub struct SpeakerMixer {
    levels: BTreeMap<AudioLeaseId, u8>,
    grants: BTreeSet<AudioLeaseId>,
    max_consumers: usize,
}

impl SpeakerMixer {
    /// Construct a mixer with a bounded number of consumers.
    pub fn new(max_consumers: usize) -> Self {
        assert!(max_consumers > 0);
        Self {
            levels: BTreeMap::new(),
            grants: BTreeSet::new(),
            max_consumers,
        }
    }

    /// Grant or revoke one speaker consumer.
    ///
    /// The return value is true when the aggregate speaker grant changed
    /// from no consumers to at least one consumer, or back to none.
    pub fn set_grant(
        &mut self,
        lease: AudioLeaseId,
        on: bool,
    ) -> Result<bool, AudioAuthorityError> {
        if on {
            if !self.grants.contains(&lease)
                && !self.levels.contains_key(&lease)
                && self.consumer_count() >= self.max_consumers
            {
                return Err(AudioAuthorityError::ConsumerLimit);
            }
            let was_empty = self.grants.is_empty();
            self.grants.insert(lease);
            Ok(was_empty)
        } else {
            let was_last = self.grants.len() == 1 && self.grants.contains(&lease);
            self.grants.remove(&lease);
            Ok(was_last)
        }
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
    pub fn set_level(&mut self, lease: AudioLeaseId, level: u8) -> Result<(), AudioAuthorityError> {
        self.can_set_level(lease, level)?;
        self.levels.insert(lease, level);
        Ok(())
    }

    /// Check whether a bounded consumer level can be admitted.
    pub(crate) fn can_set_level(
        &self,
        lease: AudioLeaseId,
        level: u8,
    ) -> Result<(), AudioAuthorityError> {
        if level > 100 {
            return Err(AudioAuthorityError::LevelOutOfRange);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAuthorityError {
    /// Level was outside 0..=100.
    LevelOutOfRange,
    /// Consumer bound was exceeded.
    ConsumerLimit,
}

impl core::fmt::Display for AudioAuthorityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::LevelOutOfRange => "audio-level-out-of-range",
            Self::ConsumerLimit => "audio-consumer-limit",
        })
    }
}

impl std::error::Error for AudioAuthorityError {}
