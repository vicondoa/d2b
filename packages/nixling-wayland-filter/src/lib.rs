//! Library entry point re-exporting public types for testing.
//!
//! The binary (`main.rs`) uses these modules directly. Tests in `tests/`
//! import through this crate root.

pub mod diag;
pub mod dmabuf;
pub mod filter;
pub mod policy;

pub use policy::{FilterPolicy, GlobalAction, PolicyInput, PolicyWarning};
