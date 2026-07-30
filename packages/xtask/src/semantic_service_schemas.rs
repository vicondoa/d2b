//! The semantic Service and Binding schema artifact generator.
//!
//! `ADR-046-provider-model-and-packaging` work item `ADR046-provider-004`
//! defines one shared base spec, status, and schema contract per frozen
//! semantic Service/Binding pair, and the generated schema artifacts for the
//! eight exact qualified ResourceTypes those bases describe. This module is
//! where that generator lands.
//!
//! Those qualified types are installed from signed Provider schemas and are
//! deliberately not members of the standard ResourceType registry in
//! `zone_schema`, which is closed.
//!
//! It is a stub. It writes nothing and decides nothing about the emitted
//! shape; the implementing slice owns this file exclusively.

use std::path::{Path, PathBuf};

/// Generate the committed schema artifacts for the semantic Service and
/// Binding bases.
///
/// Unimplemented: returns an error naming the work item that fills it, so a
/// caller cannot mistake an empty result for an up-to-date artifact set.
pub fn gen_semantic_service_schemas(
    _repo_root: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    Err(
        "gen-semantic-service-schemas is not implemented yet; it is filled by ADR046-provider-004"
            .into(),
    )
}
