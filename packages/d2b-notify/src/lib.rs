// SPDX-License-Identifier: Apache-2.0
//! Reusable notification/event mechanism for d2b desktop UX.
//!
//! This crate provides:
//!
//! - **[`events`]**: typed security-key event enum consumed by the desktop
//!   notification layer and the Waybar helper.
//! - **[`nonce`]**: single-use CSPRNG action nonces bound to
//!   session/action/expiry, preventing notification-action replay from
//!   other desktop clients.
//! - **[`notifications`]**: `Notification` struct, pluggable `Notifier` trait,
//!   and per-event notification builders with optional action payloads.
//! - **[`state`]**: durable JSON state format written by the host runtime and
//!   read by the Waybar helper and `d2b-wlcontrol`.
//! - **[`waybar`]**: Waybar JSON-protocol block helper (`text`/`tooltip`/`class`).
//! - **[`wlcontrol`]**: data contract for the `d2b-wlcontrol` status/action
//!   surface (consumed by wlcontrol; produced by the host runtime).

pub mod events;
pub mod nonce;
pub mod notifications;
pub mod state;
pub mod waybar;
pub mod wlcontrol;
