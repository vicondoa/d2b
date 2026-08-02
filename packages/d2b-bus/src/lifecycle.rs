//! ComponentSession lifecycle used by the Zone bus.
//!
//! The session implementation is owned by `d2b-session`; this module is the
//! bus-facing canonical path so callers do not grow a second lifecycle FSM.

pub use d2b_session::{KeepaliveAction, SessionLifecycle, SessionPhase};
