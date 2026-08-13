//! Bounded owner-Service audio authority.

use std::collections::{BTreeMap, VecDeque};

/// Opaque operation-scoped audio lease identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioLeaseId(u64);

impl AudioLeaseId {
    /// Construct a non-zero lease identity.
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
    queue: VecDeque<(AudioLeaseId, String)>,
    max_queue: usize,
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
    pub fn request(&mut self, lease: AudioLeaseId, zone: impl Into<String>) -> MicDecision {
        if self.active == Some(lease) || self.queue.iter().any(|(id, _)| *id == lease) {
            return MicDecision::Granted;
        }
        if self.active.is_none() {
            self.active = Some(lease);
            MicDecision::Granted
        } else if self.queue.len() >= self.max_queue {
            MicDecision::QueueFull
        } else {
            self.queue.push_back((lease, zone.into()));
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
        self.queue.retain(|(id, _)| *id != lease);
        before != self.queue.len()
    }

    /// Mute-before-handoff and grant the next FIFO lease.
    pub fn next_lease(&mut self) -> Option<AudioLeaseId> {
        if self.active.is_some() {
            return self.active;
        }
        self.active = self.queue.pop_front().map(|(lease, _)| lease);
        self.active
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
    max_consumers: usize,
}

impl SpeakerMixer {
    /// Construct a mixer with a bounded number of consumers.
    pub fn new(max_consumers: usize) -> Self {
        assert!(max_consumers > 0);
        Self {
            levels: BTreeMap::new(),
            max_consumers,
        }
    }

    /// Set a bounded consumer level.
    pub fn set_level(&mut self, lease: AudioLeaseId, level: u8) -> Result<(), AudioAuthorityError> {
        if level > 100 {
            return Err(AudioAuthorityError::LevelOutOfRange);
        }
        if !self.levels.contains_key(&lease) && self.levels.len() >= self.max_consumers {
            return Err(AudioAuthorityError::ConsumerLimit);
        }
        self.levels.insert(lease, level);
        Ok(())
    }

    /// Remove one consumer.
    pub fn remove(&mut self, lease: AudioLeaseId) {
        self.levels.remove(&lease);
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
