//! `d2b-wlattach` — reconnectable Wayland application forwarding (prototype).
//!
//! *tmux for GUI apps.* A persistent **session host** owns the application's
//! Wayland connection and all of its surface state. A disposable **window
//! frontend** connects to the real compositor and can be detached and
//! re-attached at will; the application never notices and never restarts.
//!
//! This crate is a standalone spike. It is intentionally not a member of the
//! d2b workspace, and it changes no shipping d2b component.
//!
//! # Layout
//!
//! * [`model`] — pure state machines. No file descriptors, no Wayland types.
//! * [`wire`] — the session-host/frontend protocol: DTOs and the
//!   `SOCK_SEQPACKET` + `SCM_RIGHTS` transport.

pub mod model;
pub mod present;
pub mod serve;
pub mod wire;
