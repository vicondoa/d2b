//! Store-view mode: the per-Guest closure-only hardlink farm.
//!
//! The guest is served the farm at `live/` and is never served the host
//! store. The effect adapter owns the layout and posture checks; this module
//! carries only the read-only evidence the controller consumes.

/// Read-only evidence for the store-view launch marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreViewMarkerEvidence {
    /// The marker file exists.
    pub present: bool,
    /// The marker file has zero length.
    pub zero_length: bool,
}
