// SPDX-License-Identifier: Apache-2.0
//! Authenticated clipboard control, descriptor bridge, and picker services.
//!
//! [`services::ClipboardServices`] is the composition boundary. It starts the
//! three bounded services transactionally from established ComponentSession
//! evidence and destroys their state and held descriptors together on session
//! loss.

#![forbid(unsafe_code)]

pub mod audit;
pub mod daemon;
pub mod fallback;
pub mod fd;
pub mod framing;
pub mod host;
pub mod niri;
pub mod notifications;
pub mod picker;
pub mod policy;
pub mod protocol;
pub mod services;
pub mod virtual_keyboard;
pub mod wayland;
