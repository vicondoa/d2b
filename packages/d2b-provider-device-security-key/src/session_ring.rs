//! Bounded recent-session ring.

use core::fmt;
use std::collections::VecDeque;

use crate::{MAX_SESSION_RING_SIZE, MIN_SESSION_RING_SIZE, SecurityKeySessionId};

/// Terminal result retained in the session ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResult {
    /// The operation is still active.
    InProgress,
    /// CTAP completed successfully.
    Success,
    /// CTAP returned an error.
    CtapError,
    /// The bounded session timeout fired.
    Timeout,
    /// The operator or owner cancelled the operation.
    Cancelled,
    /// A transport/effect failure occurred.
    InternalError,
}

/// One redacted session record.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionRecord {
    id: SecurityKeySessionId,
    result: SessionResult,
}

impl SessionRecord {
    /// Construct a record without CTAP bytes, RP data, paths, or holder names.
    pub const fn new(id: SecurityKeySessionId, result: SessionResult) -> Self {
        Self { id, result }
    }

    /// Return the opaque session ID.
    pub const fn id(&self) -> SecurityKeySessionId {
        self.id
    }

    /// Return the closed result.
    pub const fn result(&self) -> SessionResult {
        self.result
    }
}

impl fmt::Debug for SessionRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionRecord")
            .field("id", &self.id)
            .field("result", &self.result)
            .finish()
    }
}

/// Bounded FIFO ring that evicts the oldest session first.
pub struct SessionRing {
    capacity: usize,
    entries: VecDeque<SessionRecord>,
}

impl SessionRing {
    /// Construct a ring in the frozen 8..=256 bound.
    pub fn new(capacity: usize) -> Result<Self, SessionRingError> {
        if !(MIN_SESSION_RING_SIZE..=MAX_SESSION_RING_SIZE).contains(&capacity) {
            return Err(SessionRingError::CapacityOutOfRange);
        }
        Ok(Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        })
    }

    /// Append a record and return the evicted oldest record, if any.
    pub fn push(&mut self, record: SessionRecord) -> Option<SessionRecord> {
        let evicted = (self.entries.len() == self.capacity)
            .then(|| self.entries.pop_front())
            .flatten();
        self.entries.push_back(record);
        evicted
    }

    /// Return the configured capacity.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Borrow records from oldest to newest.
    pub fn entries(&self) -> impl Iterator<Item = &SessionRecord> {
        self.entries.iter()
    }
}

impl fmt::Debug for SessionRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionRing")
            .field("capacity", &self.capacity)
            .field("len", &self.entries.len())
            .finish()
    }
}

/// Session ring construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRingError {
    /// The requested capacity is outside the frozen bound.
    CapacityOutOfRange,
}

impl fmt::Display for SessionRingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("security-key-session-ring-capacity-out-of-range")
    }
}

impl std::error::Error for SessionRingError {}
