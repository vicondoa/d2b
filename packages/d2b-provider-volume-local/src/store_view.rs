//! Store-view mode: the per-Guest closure-only hardlink farm.
//!
//! The guest is served the farm at `live/` and is never served the host
//! store. The effect adapter owns the layout and posture checks; this module
//! carries only the read-only evidence the controller consumes.

/// The store-view Volume row name prefix the plane resolves for a Guest.
///
/// The daemon reads this prefix to recognize a Guest's closure store-view
/// Volume instead of spelling the family's row shape itself.
pub const STORE_VIEW_VOLUME_NAME_PREFIX: &str = "store-view-";

/// Read-only evidence for the store-view launch marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreViewMarkerEvidence {
    /// The marker file exists.
    pub present: bool,
    /// The marker file has zero length.
    pub zero_length: bool,
}
