//! What the pre-v3 plane still reads from the Process family.
//!
//! The Process/EphemeralProcess reconciliation itself lives in the family's
//! own crate, together with the one canonical launch-identity resolver every
//! ticket consumer shares. What stayed here was the restart annotation the
//! daemon stamped on the display workers' `Process` rows; the display
//! Provider derives and stamps that annotation itself now that it owns the
//! child rows, so the daemon keeps no copy of the vocabulary.
