//! Validator evidence import (spec section 12.2, work item
//! `ADR046-delivery-003`).
//!
//! Contract this module will satisfy:
//!
//! * Import command and result evidence from the three concurrent lanes:
//!   required GitHub CI, the heavy-gated local and host validators, and the
//!   ten-role panel.
//! * Key every record by the snapshot's `candidate_id` and reject evidence
//!   bound to a stale candidate.
//! * Store records only through
//!   [`CandidateDir`](super::storage::CandidateDir), so raw validator output
//!   never lands in a tracked file, a generated artifact, or a pull-request
//!   body.
//! * Report a lane as pending rather than passing while its result is
//!   outstanding; a pending lane never permits merge.
//!
//! Until it lands, `cargo xtask delivery wave validate-import` fails closed.
