//! Wave seal construction (spec section 12.4, work item
//! `ADR046-delivery-006`).
//!
//! Contract this module will satisfy:
//!
//! * Require all ten panel records present, unanimous, and bound to the same
//!   `candidate_id`, `content_id`, and `snapshot_sha256`.
//! * Require every validator lane from spec section 12.2 to report success on
//!   that exact snapshot.
//! * Reject the seal when any record is missing, mismatched, or bound to a
//!   different candidate.
//!
//! Until it lands, `cargo xtask delivery wave seal` fails closed.
