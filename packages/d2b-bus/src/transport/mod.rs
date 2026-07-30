//! Zone bus transport.
//!
//! [`credit`] owns the Zone-level attachment credit contract: which route
//! classes may carry descriptors at all, and how the per-scope pools that bound
//! them are sized. [`unix`] owns admission of an allocator-issued Unix
//! descriptor into an `OwnedTransport` the session engine can consume.

pub mod credit;
pub mod unix;
