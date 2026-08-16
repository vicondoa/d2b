//! Supervisor-owned bounded terminal output ring.

use std::collections::VecDeque;

use crate::ShellTerminalError;

/// In-memory merged terminal output retained only by one session supervisor.
pub struct OutputRing {
    capacity: usize,
    bytes: VecDeque<u8>,
    evicted_bytes: u64,
}

impl OutputRing {
    /// Create a ring within the provider's documented capacity bounds.
    pub fn new(capacity: usize) -> Result<Self, ShellTerminalError> {
        if !(4096..=1024 * 1024).contains(&capacity) {
            return Err(ShellTerminalError::CapacityOutOfRange);
        }
        Ok(Self {
            capacity,
            bytes: VecDeque::with_capacity(capacity),
            evicted_bytes: 0,
        })
    }

    /// Append terminal bytes, evicting only oldest bytes when the ring is full.
    pub fn append(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.bytes.len() == self.capacity {
                self.bytes.pop_front();
                self.evicted_bytes = self.evicted_bytes.saturating_add(1);
            }
            self.bytes.push_back(*byte);
        }
    }

    /// Return the number of bytes currently retained.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Return whether the ring has no retained bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return total bytes evicted since supervisor start.
    pub const fn evicted_bytes(&self) -> u64 {
        self.evicted_bytes
    }

    /// Copy the last bounded number of bytes for an attach replay.
    pub fn tail(&self, requested: usize) -> RingReplay {
        let start = self.bytes.len().saturating_sub(requested);
        RingReplay {
            bytes: self.bytes.iter().skip(start).copied().collect(),
            truncated: start != 0,
        }
    }
}

impl std::fmt::Debug for OutputRing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutputRing")
            .field("capacity", &self.capacity)
            .field("len", &self.bytes.len())
            .field("evicted_bytes", &self.evicted_bytes)
            .finish()
    }
}

/// A copied attach replay whose debug output never includes terminal bytes.
pub struct RingReplay {
    bytes: Vec<u8>,
    truncated: bool,
}

impl RingReplay {
    /// Borrow the replay bytes for the authenticated named terminal stream.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return whether the request omitted older bytes still present in the ring.
    pub const fn was_truncated(&self) -> bool {
        self.truncated
    }
}

impl std::fmt::Debug for RingReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RingReplay")
            .field("len", &self.bytes.len())
            .field("truncated", &self.truncated)
            .finish()
    }
}
