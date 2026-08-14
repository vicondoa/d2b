//! Compatibility re-export for the pure Cloud Hypervisor argv contract.
//!
//! The implementation lives in `d2b-host-argv` so Provider code can reuse
//! the canonical generator without depending on host mutation APIs.

pub use d2b_host_argv::*;
