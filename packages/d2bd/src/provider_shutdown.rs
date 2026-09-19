//! Provider-aware graceful VM shutdown seam for daemon lifecycle code.
//!
//! The shutdown vocabulary and the Cloud Hypervisor adapter live in the
//! crate that owns the guest runtime families (U7); the daemon reads them
//! from there and implements the qemu-media leg (which rides the broker's
//! QMP surface) behind the same seam in `composition.rs`.

pub use d2b_provider_guest::shutdown::{
    GracefulVmShutdown, ProviderGuestState, ProviderKind, ProviderRequestOutcome,
    ProviderShutdownTarget, ProviderVmmExitOutcome,
};