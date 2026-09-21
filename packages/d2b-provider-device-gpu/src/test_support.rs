//! Test-support doubles for the GPU effects facet set.
//!
//! Gated behind the `test-support` Cargo feature (or `cfg(test)`) so
//! production consumers never pull this in; `d2bd`'s plane tests build a
//! facet set through this module.

use std::sync::Arc;

use d2b_core_controller::authority::{AuthorityLease, AuthorityRequest};

use crate::effects::GpuEffectError;
use crate::facets::{GpuEffectFacets, GpuRuntime};

/// A fail-closed [`GpuRuntime`] double: every authority read refuses closed
/// by name, so the plane tests never silently pass a port that would have
/// reached daemon state.
pub struct FailClosedRuntime;

impl GpuRuntime for FailClosedRuntime {
    fn admit_authority(&self, _request: AuthorityRequest) -> Result<AuthorityLease, GpuEffectError> {
        Err(GpuEffectError::AuthorityConflict)
    }

    fn release_authority(&self, _lease: &AuthorityLease) -> Result<(), GpuEffectError> {
        Err(GpuEffectError::AuthorityConflict)
    }
}

/// Build a GPU facet set from the fail-closed runtime double.
pub fn recording_facets() -> GpuEffectFacets {
    GpuEffectFacets {
        runtime: Arc::new(FailClosedRuntime),
    }
}