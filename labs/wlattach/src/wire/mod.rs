//! The session-host ↔ window-frontend protocol.
//!
//! [`dto`] holds the message types; [`transport`] holds the datagram/descriptor
//! machinery. The split is deliberate: DTOs are pure data and can be tested
//! without touching a socket.

pub mod dto;
pub mod transport;
