//! Ten-role panel request and attestation (spec section 12.3, work item
//! `ADR046-delivery-005`).
//!
//! Contract this module will satisfy:
//!
//! * `panel-request` writes the candidate-bound request naming exactly the ten
//!   roles in [`PANEL_ROLES`](super::model::PANEL_ROLES) and the required
//!   provider and model from
//!   [`PANEL_PROVIDER_POLICY`](super::model::PANEL_PROVIDER_POLICY) and
//!   [`PANEL_MODEL_POLICY`](super::model::PANEL_MODEL_POLICY).
//! * `panel-attest` validates a directory holding exactly one strict record
//!   per role, each bound to the same `candidate_id`, `content_id`, and
//!   `snapshot_sha256`, rejecting a wrong model, a missing or duplicate role,
//!   duplicate run provenance, or a `signoff` inconsistent with
//!   `recommendations`.
//! * `signoff` is true if and only if `recommendations` is empty. Any finding
//!   requires a content change, which creates a new snapshot and invalidates
//!   every prior record for the wave.
//! * Provider and model fields exist only inside the external delivery-state
//!   directory. They never reach a committed file, a pull-request body, or a
//!   release archive.
//!
//! Until it lands, `cargo xtask delivery wave panel-request` and
//! `panel-attest` fail closed.
