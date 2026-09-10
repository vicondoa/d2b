//! Explicit Host vs Guest execution-target layer (R19).
//!
//! Unit U4 establishes the target handle every driver context exposes; U13
//! fills the target directory, the guest session rename, and target-local
//! discovery/adoption on top of this handle.

pub const MODULE_NAME: &str = "target";

/// Where a resource's effects execute (spec section 23.1). A Host-zone
/// resource may realize inside a guest while remaining owned and visible in
/// the Host zone (R18); the handle carries only the execution target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetHandle {
    /// Effects run locally on the host.
    Host,
    /// Effects run inside a guest through the authenticated ComponentSession.
    Guest,
}