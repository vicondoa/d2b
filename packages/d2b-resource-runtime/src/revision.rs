//! Runtime revisions (daemon epoch + sequence) for watch cursors (U5).
//!
//! Watches are cursor-addressed by [`RuntimeRevision`], an opaque
//! lexicographically ordered (epoch, sequence) pair. Every desired or
//! runtime change bumps the sequence within the current daemon epoch
//! (R23); a cursor from an older epoch is rejected rather than replayed
//! (R24/AE4).
//!
//! # Epoch source
//!
//! The epoch is **nanoseconds-since-UNIX at daemon startup**, stamped once
//! by [`RevisionEpoch::stamp`]. Given a wall clock that only moves forward,
//! every restart stamps a strictly larger epoch, so any cursor handed out
//! during a previous daemon lifetime maps to a strictly older epoch and
//! fails closed at registration.
//!
//! *Clock-warp caveat:* if the wall clock is moved backward across a
//! restart (past the previous startup instant), the new epoch can be equal
//! to or older than a prior lifetime's epoch. The hub treats an
//! equal-or-older epoch registration as expired - within one lifetime the
//! epoch is constant, so a genuinely stale cross-restart cursor in that
//! degenerate case degrades to the in-epoch sequence-gap check (a cursor
//! behind the ring's retention frontier is also rejected), which still
//! fails closed. Restarts in the same nanosecond degenerate the same way.
//!
//! # Wire mapping (implemented in U8)
//!
//! The external wire carries a single u64 `ZoneRevision`
//! (`packages/d2b-contracts-resource/src/v3/identity.rs:530`). The mapping
//! is `(epoch_seconds << 32) | sequence` where `epoch_seconds =
//! epoch_nanos / 1_000_000_000` (nanos/1e9 as u64) and `sequence` is
//! truncated to its low 32 bits. Budget: the sequence must stay below
//! [`WIRE_SEQUENCE_BUDGET`] (2^32) within one epoch, and the epoch-second
//! count must fit in 32 bits (satisfied until year 2106). The full mapping
//! lands with U8; this module only pins the budget.

use d2b_contracts_resource::v3::authority::{DesiredRevision, RevisionError, ZoneDesiredSequence};

/// The module declared name.
pub const MODULE_NAME: &str = "revision";

/// Maximum revisions per daemon epoch under the U8 wire mapping: the
/// sequence must fit in the low 32 bits of the u64 `ZoneRevision`.
pub const WIRE_SEQUENCE_BUDGET: u64 = 1 << 32;

// ---------------------------------------------------------------------------
// Durable desired counters (U5, KTD5)
// ---------------------------------------------------------------------------

/// The largest counter value the durable store persists.
///
/// Desired revisions and the Zone desired sequence are stored as SQLite
/// integers, whose signed 64-bit range is the persisted ceiling. The ceiling
/// is lower than [`u64::MAX`] on purpose: a counter that cannot be written
/// back fails closed instead of wrapping into a revision that looks older
/// than what it replaced.
pub const MAX_PERSISTED_DESIRED_COUNTER: u64 = i64::MAX as u64;

/// Advance a durable row's desired revision.
///
/// Every committed desired mutation goes through this, so an exhausted
/// counter refuses the mutation instead of committing an unorderable
/// revision.
pub fn next_desired_revision(previous: DesiredRevision) -> Result<DesiredRevision, RevisionError> {
    let next = previous.try_next()?;
    if next.get() > MAX_PERSISTED_DESIRED_COUNTER {
        return Err(RevisionError::Exhausted);
    }
    Ok(next)
}

/// Advance the durable per-Zone desired sequence.
pub fn next_zone_sequence(
    previous: ZoneDesiredSequence,
) -> Result<ZoneDesiredSequence, RevisionError> {
    let next = previous.try_next()?;
    if next.get() > MAX_PERSISTED_DESIRED_COUNTER {
        return Err(RevisionError::Exhausted);
    }
    Ok(next)
}

/// Decode one persisted counter column.
///
/// A stored value that cannot be a counter at all (a negative column) is
/// refused instead of reinterpreted: an authority change ordered against a
/// counter the store cannot account for has no meaning, and refusing keeps
/// the failure visible rather than folding it into an unrelated revision.
pub const fn decode_persisted_counter(stored: i64) -> Option<u64> {
    if stored < 0 { None } else { Some(stored as u64) }
}

/// Rebuild a durable row's desired revision from a persisted counter.
///
/// `DesiredRevision` publishes no raw-value constructor, so a stored counter
/// is read back through the type's own `Deserialize` impl - the one public
/// path from an integer to the revision - and a value the contract rejects
/// decodes to nothing rather than to a nearby revision.
pub fn desired_revision_from_counter(value: u64) -> Option<DesiredRevision> {
    serde_json::from_value(serde_json::Value::from(value)).ok()
}

/// Rebuild the durable Zone desired sequence from a persisted counter.
pub fn zone_sequence_from_counter(value: u64) -> Option<ZoneDesiredSequence> {
    serde_json::from_value(serde_json::Value::from(value)).ok()
}

/// Encode one counter for persistence.
pub const fn encode_persisted_counter(value: u64) -> Option<i64> {
    if value > MAX_PERSISTED_DESIRED_COUNTER { None } else { Some(value as i64) }
}

/// A watch-cursor revision: daemon epoch plus per-epoch sequence.
///
/// Ordered lexicographically - epoch first, then sequence - so revisions
/// from different epochs never interleave and an older epoch is always
/// strictly older.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeRevision {
    /// Startup-stamped daemon epoch (nanoseconds since UNIX at startup).
    pub epoch: u64,
    /// Monotonic change sequence within the epoch (starts at 1).
    pub sequence: u64,
}

impl RuntimeRevision {
    /// Construct a revision from its parts.
    pub const fn new(epoch: u64, sequence: u64) -> Self {
        Self { epoch, sequence }
    }

    /// The first revision handed out in `epoch` (one past the empty cursor).
    pub const fn first_of(epoch: RevisionEpoch) -> Self {
        Self {
            epoch: epoch.nanos,
            sequence: 1,
        }
    }
}

impl std::fmt::Display for RuntimeRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "e{}+{}", self.epoch, self.sequence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The daemon's revision epoch: nanoseconds-since-UNIX at startup.
pub struct RevisionEpoch {
    nanos: u64,
}

impl RevisionEpoch {
    /// Stamp the current epoch from `clock`. Called once at daemon startup.
    pub fn stamp(clock: &(impl RevisionClock + ?Sized)) -> Self {
        Self {
            nanos: clock.now_nanos(),
        }
    }

    /// The epoch identifier as carried in [`RuntimeRevision::epoch`].
    pub const fn nanos(&self) -> u64 {
        self.nanos
    }
}

/// Source of startup time, abstracted so tests can simulate epochs
/// (`ManualClock`) and prior-lifetime cursors.
pub trait RevisionClock: Send + Sync {
    /// Nanoseconds since the UNIX epoch.
    fn now_nanos(&self) -> u64;
}

/// Production clock: the host wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl RevisionClock for SystemClock {
    fn now_nanos(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before the UNIX epoch")
            .as_nanos() as u64
    }
}

/// Test clock: a manually advanced nanosecond counter.
#[derive(Debug, Default)]
pub struct ManualClock(std::sync::atomic::AtomicU64);

impl ManualClock {
    /// A clock frozen at `nanos`.
    pub fn at(nanos: u64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(nanos))
    }

    /// Advance the clock by `nanos`.
    pub fn advance(&self, nanos: u64) {
        self.0.fetch_add(nanos, std::sync::atomic::Ordering::Relaxed);
    }
}

impl RevisionClock for ManualClock {
    fn now_nanos(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ord_is_lexicographic_epoch_then_sequence() {
        let base = RuntimeRevision::new(10, 500);
        assert!(RuntimeRevision::new(9, u64::MAX) < base);
        assert!(RuntimeRevision::new(10, 499) < base);
        assert_eq!(base, RuntimeRevision::new(10, 500));
        assert!(RuntimeRevision::new(10, 501) > base);
        assert!(RuntimeRevision::new(11, 0) > base);
    }

    #[test]
    fn display_shows_epoch_and_sequence() {
        let revision = RuntimeRevision::new(1_728_000_000, 42);
        let rendered = revision.to_string();
        assert!(rendered.contains('4'), "got: {rendered}");
        assert!(rendered.contains("e1728000000+42"), "got: {rendered}");
    }

    #[test]
    fn epoch_stamp_uses_clock_nanos() {
        let clock = ManualClock::at(1_000_000_123);
        let epoch = RevisionEpoch::stamp(&clock);
        assert_eq!(epoch.nanos(), 1_000_000_123);
        // A second stamp from an advanced clock is strictly newer.
        clock.advance(5);
        assert!(RevisionEpoch::stamp(&clock).nanos() > epoch.nanos());
    }

    #[test]
    fn first_revision_of_epoch_is_sequence_one() {
        let epoch = RevisionEpoch::stamp(&ManualClock::at(7));
        assert_eq!(
            RuntimeRevision::first_of(epoch),
            RuntimeRevision::new(7, 1),
        );
        assert!(RuntimeRevision::first_of(epoch) < RuntimeRevision::new(7, 2));
    }

    #[test]
    fn wire_budget_bounds_sequence_for_u32_low_word() {
        // The U8 wire mapping packs (epoch_seconds << 32) | sequence(u32);
        // so the largest in-budget sequence must survive the low-32-bit
        // truncation intact and the epoch must stay in the high word.
        let max_sequence = WIRE_SEQUENCE_BUDGET - 1;
        let wire = (1_u64 << 32) | max_sequence;
        assert_eq!(wire as u32 as u64, max_sequence);
        assert_eq!(wire >> 32, 1);
    }
}
