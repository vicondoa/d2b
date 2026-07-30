//! The Provider crate skeleton and Nix package/catalog generator.
//!
//! `ADR-046-provider-model-and-packaging` work item `ADR046-provider-002`
//! makes the Provider crate layout and the generic Nix Provider
//! package/catalog emitter generated rather than hand-maintained. This module
//! is where that generator lands.
//!
//! It is a stub. It writes nothing and decides nothing about the emitted
//! shape; the implementing slice owns this file exclusively.

use std::path::{Path, PathBuf};

/// Generate the Provider crate skeletons and the Nix Provider
/// package/catalog artifacts.
///
/// Unimplemented: returns an error naming the work item that fills it, so a
/// caller cannot mistake an empty result for an up-to-date artifact set.
pub fn gen_provider_packaging(
    _repo_root: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    Err("gen-provider-packaging is not implemented yet; it is filled by ADR046-provider-002".into())
}
