//! Host-side Wayland proxy implementation owned by the display Provider.

pub mod attribution;
pub mod bridge;
pub mod clipboard;
pub mod decoration;
pub mod diag;
pub mod dmabuf;
pub mod filter;
pub mod identity;
pub mod policy;
pub mod readiness;
pub mod terminal;

pub use policy::{FilterPolicy, GlobalAction, PolicyInput, PolicyWarning};
