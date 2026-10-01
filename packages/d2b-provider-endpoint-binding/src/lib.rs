//! The EndpointBinding provider crate.
//!
//! Serves the `EndpointBinding` resource type: its driver, spec decoder, and the
//! realization the source-side consumers resolve against.

#![deny(missing_docs)]

mod driver;

pub use driver::{BINDING_TYPE_NAME, binding_descriptor, binding_spec_decoder};
