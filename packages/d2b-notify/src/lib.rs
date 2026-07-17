// SPDX-License-Identifier: Apache-2.0
//! Authenticated desktop observer and action services.
//!
//! [`services::DesktopServices`] is the composition boundary. It admits one
//! already-established, authenticated `desktop-observer` ComponentSession and
//! starts the bounded observer and action services together. Presentation
//! state remains a read-only projection and is never an alternate control path.

#![forbid(unsafe_code)]

pub mod events;
pub mod nonce;
pub mod notifications;
pub mod services;
pub mod state;
pub mod waybar;
pub mod wlcontrol;
