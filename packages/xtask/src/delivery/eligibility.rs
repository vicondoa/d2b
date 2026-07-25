//! Merge eligibility (spec section 12.4, work item `ADR046-delivery-006`).
//!
//! Contract this module will satisfy, per pull request in the wave's stack:
//!
//! * The seal exists for the wave's candidate.
//! * The pull request's current base and head still match the sealed
//!   snapshot's recorded object IDs, or a history-only rebase has passed the
//!   byte-identical proof in [`history_proof`](super::history_proof).
//! * Every required GitHub check is green; a missing, duplicate, pending,
//!   failed, or malformed required check fails closed.
//!
//! Until it lands, `cargo xtask delivery wave merge-eligibility` fails closed.
