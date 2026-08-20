#![doc = "Canonical guest and public control-plane wire contracts for d2b."]

pub mod cli_output;
pub mod generated;
pub mod guest_auth;
pub mod guest_wire;
pub mod public_wire;
pub mod proxy_readiness;
pub mod terminal_wire;
pub mod unsafe_local_wire;

pub mod guest_proto {
    pub use crate::generated::guest_control::*;
}
