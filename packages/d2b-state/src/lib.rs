//! Durable state primitives for d2b 2.0.
//!
//! The crate deliberately has no default feature. Host filesystem access is
//! available only with `host-fs`.

#![forbid(unsafe_code)]

#[cfg(all(feature = "host-fs", not(target_os = "linux")))]
compile_error!("the host-fs feature requires Linux");

#[cfg(feature = "host-fs")]
mod atomic;
#[cfg(feature = "host-fs")]
mod audit;
#[cfg(feature = "host-fs")]
mod error;
#[cfg(feature = "host-fs")]
mod lease;
#[cfg(feature = "host-fs")]
mod lock;
#[cfg(feature = "host-fs")]
mod path;
#[cfg(feature = "tokio")]
mod tokio_api;

#[cfg(feature = "host-fs")]
pub use atomic::{
    AtomicFilesystem, AtomicWrite, CanonicalJson, DurableState, GenerationPolicy,
    MetadataExpectation, QuarantineRecord, ReadPolicy, RealAtomicFilesystem, WritePolicy,
};
#[cfg(feature = "host-fs")]
pub use audit::{
    AuditAppender, AuditRecordInput, SegmentBuilder, checkpoint, decide_retention, detect_gap,
    read_audit_segment,
};
#[cfg(feature = "host-fs")]
pub use error::{Error, ErrorCode, Result};
#[cfg(feature = "host-fs")]
pub use lease::{LeaseStatus, grant_lease, revoke_lease, validate_lease};
#[cfg(feature = "host-fs")]
pub use lock::{Cancellation, Clock, LockGuard, LockSet, NeverCancelled, OfdTransfer, SystemClock};
#[cfg(feature = "host-fs")]
pub use path::{AnchoredDir, AnchoredResource, LeafName, RelativePath};
#[cfg(feature = "tokio")]
pub use tokio_api::{
    async_anchored_dir_from_fd, async_atomic_quarantine, async_atomic_read, async_atomic_write,
    async_audit_append, async_audit_create, async_audit_resume, async_ofd_lock_acquire,
    async_ofd_lock_acquire_with_clock, async_ofd_lock_release, async_open_anchored_dir,
};
