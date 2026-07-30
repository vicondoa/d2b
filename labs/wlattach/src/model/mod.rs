//! Pure state machines.
//!
//! Nothing in this module may hold a file descriptor, a Smithay type or a
//! wayland-client type. Adapters translate [`ledger::Effect`]s into real
//! resource operations. Keeping the boundary strict is what makes the
//! safety-critical accounting testable without a compositor or a GPU.

pub mod ids;
pub mod ledger;
