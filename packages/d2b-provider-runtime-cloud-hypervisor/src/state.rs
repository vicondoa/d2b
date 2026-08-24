//! Status-first Cloud Hypervisor state.

use std::fmt;

/// Redacted Guest runtime status.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GuestRuntimeStatus {
    /// Current Provider phase.
    pub phase: &'static str,
    /// Whether the VMM process is ready.
    pub runtime_ready: bool,
    /// Whether the Guest ComponentSession is authenticated and healthy.
    pub bootstrap_ready: bool,
    /// Active process count.
    pub active_process_count: u16,
}

impl fmt::Debug for GuestRuntimeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestRuntimeStatus")
            .field("phase", &self.phase)
            .field("runtime_ready", &self.runtime_ready)
            .field("bootstrap_ready", &self.bootstrap_ready)
            .field("active_process_count", &self.active_process_count)
            .finish()
    }
}
