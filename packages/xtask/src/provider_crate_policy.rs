//! The Provider crate layout policy check.
//!
//! `ADR-046` work item `ADR046-pstate-011` asserts that every Provider crate
//! carries the layout its packaging contract requires. This module is where
//! that check lands, and it runs as `cargo xtask check-provider-crate-layout`
//! under the existing `test-policy` lane rather than as a shell gate, because
//! the drift and meta gate set is closed.
//!
//! It is a stub. It inspects nothing and decides nothing about the layout the
//! policy requires; the implementing slice owns this file exclusively.

use std::path::Path;

/// Check every Provider crate against the required crate layout.
///
/// Unimplemented: returns an error naming the work item that fills it, so a
/// lane cannot mistake a check that inspected nothing for a tree that passes.
pub fn check(_repo_root: &Path) -> Result<(), String> {
    Err(
        "check-provider-crate-layout is not implemented yet; it is filled by ADR046-pstate-011"
            .to_string(),
    )
}
