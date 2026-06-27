//! Detached-exec capacity, quota, and retention constants shared by the
//! runner (writer) and guestd (reader/supervisor) so both agree on the exact
//! worst-case reservation. The quota invariant
//! (`quota == slots * 2 * stream_cap`) is asserted by a unit test below.

/// Maximum concurrent *running* detached execs (a concurrency cap, NOT a
/// runtime cap — each may run indefinitely).
pub const DETACHED_ACTIVE_PER_VM: usize = 8;

/// Total retained detached records (running + terminal) per VM. Equals the
/// number of slots.
pub const DETACHED_RETAINED_PER_VM: usize = 32;

/// Per-stream retained-log byte cap (drop-oldest). Bounds the tmpfs worst
/// case; advertised via `effective_limits`.
pub const DETACHED_STREAM_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// VM-global retained-log byte quota. Defined to be exactly
/// `slots * 2 streams * stream_cap` so the worst-case reservation is exact
/// and an adopted-over-budget state is structurally impossible.
pub const DETACHED_LOG_QUOTA_BYTES: u64 =
    (DETACHED_RETAINED_PER_VM as u64) * 2 * DETACHED_STREAM_LOG_BYTES;

/// Per-exec reserved retained-log bytes (both streams at the cap).
pub const DETACHED_PER_EXEC_LOG_BYTES: u64 = 2 * DETACHED_STREAM_LOG_BYTES;

/// Retention TTL after a record reaches a terminal state (TERMINAL records
/// only; a Running detached job is never reaped by TTL).
pub const DETACHED_RETENTION_TTL_MS: u64 = 30 * 60 * 1_000;

/// Bounded wait for the runner's first phase marker before a create call
/// resolves via a unit re-query.
pub const CREATE_TIMEOUT_MS: u64 = 10_000;

/// Bounded persisted dispatch deadline: a crash-recovered no-unit
/// `Dispatching` record is held in-flight (slot reserved, non-listable) until
/// this elapses with a negative unit re-query.
pub const DISPATCH_DEADLINE_MS: u64 = 30_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_invariant_holds_exactly() {
        assert_eq!(
            DETACHED_LOG_QUOTA_BYTES,
            (DETACHED_RETAINED_PER_VM as u64) * 2 * DETACHED_STREAM_LOG_BYTES,
            "quota must equal slots * 2 streams * stream cap exactly"
        );
        assert_eq!(DETACHED_LOG_QUOTA_BYTES, 256 * 1024 * 1024);
        assert_eq!(DETACHED_PER_EXEC_LOG_BYTES, 2 * DETACHED_STREAM_LOG_BYTES);
        // Per-exec reservation times slot count is exactly the global quota.
        assert_eq!(
            DETACHED_PER_EXEC_LOG_BYTES * (DETACHED_RETAINED_PER_VM as u64),
            DETACHED_LOG_QUOTA_BYTES
        );
    }
}
