//! The broker executable's handler composition root (KTD1).
//!
//! `d2b-broker` the crate links no provider crate and owns the generic
//! envelope; the broker PROCESS is a composition. This crate is where
//! provider-contributed handler crates are linked and where the mechanical
//! routing rule admits their declared operations to the in-broker handler
//! table - never in the envelope.
//!
//! This pass the seam ships fixture-exercised: no census operation maps to
//! the in-broker leg, so the production binary registers nothing and the
//! fixture tests prove the seam end to end.

#![deny(unsafe_code)]

/// The mechanical routing rule (KTD1): which committed rows the in-broker
/// table may serve, and which are refused admission to it.
pub mod routing;

/// The registration seam: declared handlers + pure certificates in, a
/// routing-checked handler table out.
pub mod seam;

/// The dependency-surface audit (U6 approach item 3): handler crates with
/// syscall-surface dependencies or entry machinery are rejected.
pub mod dependency_surface;