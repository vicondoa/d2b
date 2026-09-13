//! Controller identity contracts and manager-served resource snapshots.
//!
//! The store-driven reconcile machinery this crate existed for (`Runner`,
//! `ControllerSource`, `PendingQueue`, the reconcile result/mutation protocol,
//! and the reconcile-pass context) was deleted with the persistent resource
//! database it coordinated.

pub mod context;
pub mod contract;

pub use context::{DependencySnapshot, ResourceSnapshot};
pub use contract::ResourceKey;
