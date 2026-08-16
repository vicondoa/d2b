//! Bounded limits for the vsock Provider.

/// Maximum number of concurrent transport handles in one service process.
pub const MAX_ACTIVE_TRANSPORTS: usize = 128;
/// Maximum length of one length-prefixed transport frame.
pub const MAX_FRAME_BYTES: usize = u16::MAX as usize;
/// Minimum accepted open deadline in milliseconds.
pub const MIN_OPEN_DEADLINE_MS: u32 = 1_000;
/// Maximum accepted open deadline in milliseconds.
pub const MAX_OPEN_DEADLINE_MS: u32 = 60_000;
/// Grace period for a bridge to stop after close.
pub const CLOSE_GRACE_MS: u64 = 500;
/// Maximum number of remembered session nonces.
pub const MAX_REPLAY_ENTRIES: usize = 256;
