//! Immutable candidate snapshot creation (spec section 12.1, work item
//! `ADR046-delivery-002`).
//!
//! Contract this module will satisfy:
//!
//! * Read the wave's base commit and every open pull request head from the
//!   stack, plus the dependency graph edges and repository set.
//! * Build a [`CandidateMaterial`](super::model::CandidateMaterial) and derive
//!   its `content_id`, `candidate_id`, and `snapshot_sha256` through
//!   [`CandidateMaterial::digests`](super::model::CandidateMaterial::digests).
//! * Write `snapshot.json` into the candidate directory supplied by
//!   [`StateRoot::candidate`](super::storage::StateRoot::candidate); the
//!   snapshot is immutable once written.
//! * Any content change afterwards invalidates both validator and panel
//!   evidence, so the wave re-snapshots and both lanes rerun.
//!
//! Until it lands, `cargo xtask delivery wave snapshot` fails closed.
