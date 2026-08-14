//! Compatibility re-export for the pure Cloud Hypervisor argv contract.
//!
//! The implementation lives in `d2b-host-argv` so Provider code can reuse
//! the canonical generator without depending on host mutation APIs.

pub use d2b_host_argv::{
    ChArgvError, ChArgvInput, ChFsShare, ChNetHandoff, ChNetIface, ChVsock, exec_arg0,
};

pub fn generate_ch_argv(input: &ChArgvInput) -> Result<Vec<String>, ChArgvError> {
    d2b_host_argv::generate_ch_argv(input)
}
