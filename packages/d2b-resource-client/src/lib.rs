//! Zone resource client.
//!
//! This crate is the caller-side half of the v3 Zone resource plane. It answers
//! two questions and nothing else:
//!
//! 1. **Where does this Zone-addressed target live?** [`RouteTable`] resolves a
//!    [`TargetInput`] to an exact [`ResolvedTarget`]: one Zone-scoped owner, one
//!    carriage class, one service. Resolution is exact and fail-closed - an
//!    unknown owner, an unadmitted carriage, an unspecified carriage, and an
//!    ambiguous table each refuse with their own [`ClientError`].
//! 2. **May this call proceed, and may it be retried?** [`CallDriver`] enforces
//!    the request lifetime, the idempotency requirement, the bounded retry
//!    budget, and cancellation, returning a typed [`AttemptDisposition`] after
//!    every attempt.
//!
//! Addressing is by Zone path throughout. No public item here carries a uid, a
//! gid, a socket path, a host path, a store path, or a descriptor, and every
//! `Debug` rendering of an identity-bearing type is redacted.
//!
//! # Authority
//!
//! An authorization verdict is an **input**. The client never mints, infers, or
//! presents authority: a peer refusal arrives as [`ClientError::Remote`]
//! carrying the canonical [`d2b_contracts::v3::ResourceErrorKind`] and
//! [`d2b_contracts::v3::RetryClass`], and a `Never` or `Reauthorize` verdict is
//! terminal regardless of the remaining retry budget.
//!
//! # What this crate deliberately does not contain
//!
//! Session establishment, the Noise handshake, wire encoding, attachments, and
//! named streams are not here. Those depend on the Zone session engine and the
//! v3 session contract module, which land separately; `d2b-bus` composes them
//! with the route and call policy this crate provides. Consequently there is no
//! connector trait, no `ConnectedClient`, and no `Response` type yet: inventing
//! one would fix a session contract this crate has no authority to define.
//!
//! The request-metadata bounds in [`call`] are carried over from the ADR45
//! client because the v3 session contract module does not publish them yet.
//! They must be reconciled against that module once it lands.

mod call;
mod client;
mod dispatch;
mod error;
mod target;

pub use call::{
    CallOptions, CancellationToken, MAX_CORRELATION_ID_BYTES, MAX_IDEMPOTENCY_KEY_BYTES,
    MAX_REQUEST_LIFETIME_MS, MAX_RETRY_ATTEMPTS, MetadataInput, REQUEST_ID_BYTES, RetryPolicy,
    SystemClock, TRACE_ID_BYTES, WallClock,
};
pub use client::ResourceClient;
pub use dispatch::{AttemptDisposition, AttemptTicket, CallDriver, MethodProfile, SessionFailure};
pub use error::ClientError;
pub use target::{
    ResolvedTarget, RouteRecord, RouteTable, ServiceOwner, TargetInput, TargetResolver,
    TransportKind, TransportSelection, ZoneServiceKind,
};
