//! Byte-identical history proof (spec section 12.6, work item
//! `ADR046-delivery-006`).
//!
//! Contract this module will satisfy:
//!
//! * A history-only rebase or retarget may reuse prior panel records only when
//!   the old and new histories carry byte-identical integrated content,
//!   byte-identical generated artifacts, and a byte-identical dependency diff
//!   and repository set.
//! * The proof rests on
//!   [`ContentId`](super::model::ContentId), which excludes commit history by
//!   construction, so a rebase that preserves content reproduces it exactly.
//! * The proof preserves the panel record only. Required CI still reruns on
//!   the new history.
//!
//! There is no `wave history-proof` subcommand of its own; the proof is an
//! input to `merge-eligibility`.
